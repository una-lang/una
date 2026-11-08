# 0123. HTTP servers: the `HttpServer` capability, granted by address, with limits, strict HTTP/1.1 and graceful shutdown

- Status: Accepted
- Date: 2026-10-19
- Deciders: Una project (accepted on 2026-10-19)
- Resolves: issue #897
- Related:
  - ADR 0116 (long-running programs: `for ever`, `on stop`, stop points, interruptible operations, the timing label `τ`, virtual time), whose examples name "the capability of #897";
  - ADR 0117 (lightweight tasks, `within`, `tasks` and `start`, limits on grants, the reactor), whose rule 3 lets #897 mark the operations that park;
  - ADR 0074 and `spec/24` (the `Http` client, its grants, its parser and its TLS);
  - ADR 0121 (draft, issue #912: the terms of edition `0.4`), whose Question 1 this record depends on;
  - ADR 0122 (draft, issue #893: `ita.una`), whose grant lines and sections will hold the forms of rules 7 and 16;
  - issue #898 (labels on `Clock` readings).
- Target: edition `0.4` (ADR 0116 rule 0). Editions `0.1` to `0.3` do not change.
- Amends:
  - **Capabilities and flow:** `spec/05 §7` (a host capability granted by address), `§8` (limits on a second capability); `spec/07 §1` (out of model);
  - **Names:** `spec/10 §11` rule 10 (names of `std.host` that only an import brings into scope);
  - **Dynamic semantics:** `spec/11 §5` (the `unanswered` event, and the reduced form of server events, which a recording for replay keeps too);
  - **Diagnostics and project files:** `spec/13 §4` (one code), `§5` (the `serve:` grant and flags), `§6` (the report), `§7` (the `[serve]` table);
  - **Standard library:** `spec/14 §3` (`HttpServer`), `§4` (`test_request`, `test_exchange`);
  - **Native backend:** `spec/16 §3` (flags), `§5` (scripted connections in the equivalence tests);
  - **Accepted records:** ADRs 0116 and 0117, by the dated amendments of Appendix A.

  These rules would live in a new chapter, `spec/31-http-servers.md`.

## Questions for acceptance

Each question changes what a programmer writes or can rely on. The project accepted this record on 2026-10-19, taking the recommendation of each question; each states its decision, and the text below follows them.

1. **One server capability, for HTTP only, now.**
   - **The choice.** (a) A host capability `HttpServer`, which reads HTTP/1.1 requests and writes replies, with the host doing all parsing, framing and TLS. (b) Also a lower-level `Listener` that hands the program raw TCP connections.
   - **Cost to the programmer of (a):** no raw TCP, so no custom binary protocols, WebSockets or HTTP/2 yet. Request and reply bodies are `Text`, so a body that is not UTF-8 is refused by the host with `400` before the program sees it (rule 10): binary uploads wait for a type of bytes, and so do ordinary file uploads through `multipart/form-data` with binary parts.
   - **Cost of (b):** Una has no type of bytes and no streams (ADR 0074 H1, H4), so a `Listener` would hand the program `Text` read from a socket, and every server would parse HTTP in Una code, where request smuggling is easy to get wrong.
   - **Recommendation:** (a). `Listener` comes with a later record on bytes and streams, which also lifts the UTF-8 limit.
   - **Decided:** (a). One host capability, `HttpServer`, for HTTP/1.1 only; no `Listener` in this record.

   ```una
   pub main() -> Unit
     uses @server: HttpServer[next, reply, close]   -- yes, in this record
     uses @raw: Listener[accept]                     -- no: a later record
   ```

2. **Two ways to write a server, and which one to teach first.**
   - **This record:** both. (a) The host-driven form: a `[serve]` table names a module whose function `handle(request) -> Reply` answers one request; the host runs the loop, a task per request, the limits and the shutdown (rule 16). (b) The language form: a `diverges` function with a `for ever` loop over `@server.next()`, `tasks` and `start` per request, and `@server.reply` (rule 15).
   - **Cost to the programmer:** (a) needs no loop, no `diverges` and no reply right. It is the faster path: a request is read, handled and answered on the core that owns its connection, with no hand-off of the request between threads; the grant's limits still cost a few atomic operations on state the cores share (rule 19). It has no time limit per request in code by default, since a `within` would stop a `handle` that reads `secret` data from replying at all (rule 16); the host's `answer` limit still bounds what each client waits. (b) is for servers that need their own loop, such as one that paces itself or combines a server with other work; it adds two hand-offs between threads per request.
   - **Recommendation:** accept both, and teach (a) first.
   - **Decided:** both forms, with the host-driven form (a), `[serve]`, taught first.

   ```una
   module health                                       -- (a): [serve] module = "health"

   pub handle(request: internal Request) -> internal Reply
   =
     Reply(status: 200, headers: [], body: "ok")
   ```

3. **TLS in this record, with its keys in the grant, and TLS 1.3 only by default.**
   - **This record:** yes. A grant `serve:https://...` names a certificate chain and a private key file, which the host loads before `main`. The key never enters the program, its values, its traces or its labels, as `Keys` already promises (`spec/14 §3`). The host opens the key file once and checks the open file: a regular file, owned by the user the run runs as or by root, writable by its owner only, readable by its group only when that group is one of the run's groups, and with no permission for others; otherwise the run does not start (rule 9). Through the `rustls` crate that `Http` already uses (ADR 0074 H7), with no early data. A plain `serve:http://...` grant stays possible on any address, for a server behind a proxy that ends TLS, and the report lists it. Client certificates and reloading a certificate without a restart wait for a later record.
   - **Sub-question: TLS 1.3 only by default?** (a) Recommended: yes. TLS 1.2 is then an opt-in item, `tls12`, which also requires the extended master secret. (b) TLS 1.2 and 1.3 by default. Cost of (a): clients that cannot speak TLS 1.3, such as some older embedded and enterprise clients, fail to connect until the grant says `tls12`.
   - **Not through `Keys`.** `Keys` holds HMAC keys in base64 and offers an operation to the program. A TLS key is used only by the host, in another format, so a grant field is simpler and gives the program no operation on it.
   - **Cost to the programmer:** a key file with mode `0644`, or any mode with a bit for others, is refused. Kubernetes mounts secrets as symbolic links to files owned by root, with `defaultMode` `0644` unless set:
     - **A pod that runs as a non-root user**, the hardened setup, works with `fsGroup` set and `defaultMode: 0440`: the file is owned by root, its group is the `fsGroup`, which Kubernetes adds to the process's groups, and only the group may read it. The default `0644` is refused.
     - **A pod that runs as root** works with `defaultMode: 0400` (or `0440`).
     - **A key copied into an image** must be owned by the user the container runs as (`runAsUser`), with mode `0400` or `0600`, or `0440` with a group of that user.
     
     On Windows, `https` grants are refused until a later record checks the file's access list.
   - **Recommendation:** accept, with (a).
   - **Decided:** TLS in this record, with its keys in the grant, and (a): TLS 1.3 only by default, TLS 1.2 only through the `tls12` item, which requires the extended master secret.

   ```toml
   [run]
   grants = ["serve:https://0.0.0.0:8443;certificate=tls/chain.pem;key=tls/key.pem;hosts=shop.example"]
   ```

4. **What peers send is input, even though the server's own speed shapes it: a deliberate exception to ADR 0117 IF4.**
   - **The issue.** ADRs 0116 and 0117 label every place where timing becomes a value. A server's requests depend on its timing twice over. A peer can measure how fast replies come and choose what to send next. And, closer to home, when the server is slow the host itself refuses requests (full `waiting`, full buffers, `connections` reached: rule 17), which changes which requests the program sees, on the timing of the program's own handlers. ADR 0117 IF4 labels exactly this kind of dependency for limited `Http` grants: `RateLimited` carries the timing label of every site that uses the grant. A request is `internal`, so a server whose handlers compute on `secret` data could, in principle, let `internal` code learn something about that data's timing.
   - **(a) Recommended:** the request stream is input from outside, and its dependence on the server's timing, including the refusals the host makes on the program's own timing, is out of model, as timing already is (`spec/07 §1`). This is a deliberate exception to the precedent of ADR 0117 IF4, not the same position as for `Http`, and the report says so for every package that serves (IF5). Public outputs stay unaffected: the accept loop branches only on whether a request came, and how many iterations ran before a stop is an input of the stop schedule (ADR 0116, Information flow), so no timing reaches a `public` value; what may reach `internal` values is which requests arrive.
   - **(b)** Follow ADR 0117 IF4: require the timing label of every handler, joined with `τ_conc`, to be at most `internal` (U4001).
   - **Cost to the programmer of (b):** a server whose request tasks read a `secret` value anywhere, such as a password hash or a medical record, could not be written at all, since the queue that refuses requests is not optional. (a) costs nothing in code. Its cost is a stated gap: the model does not bound what `internal` code, or a peer, learns from a server's timing.
   - **Recommendation:** (a).
   - **Decided:** (a). The request stream is input from outside, as a deliberate exception to ADR 0117 IF4, stated in the report (IF5).

   ```una
   let next = attempt @server.next()      -- internal Exchange: input from outside
   -- (b) would make this U4001 in any server whose handlers read `secret` data
   ```

5. **A request that the program never answers.**
   - **This record:** every request comes with a **reply right**, held by one task at a time. `@server.next()` gives it to the calling task, and `start` moves it to the started task that captures it (rule 4). When a task ends still holding a reply right, the host answers for it: `500` when the task returned or ended alone, `503` when it was cancelled or stopped. A second reply to one reply right, or a reply by a task that does not hold it, fails with `NotHeld`. A reply that comes after the host's own deadline for the request (rule 7, `answer`) is discarded and returns `unit`, so no value of the program depends on that timing.
   - **Cost to the programmer:** a forgotten reply is a `500` at the end of the task, not a hang. The alternative, a static rule that every reply right is used exactly once (a linear type), would need a new kind of type for one capability.
   - **Recommendation:** accept.
   - **Decided:** yes. Reply rights held by one task at a time, with auto-answers at a task's end (rule 4).

   ```una
   start forward(exchange) within 5 * second    -- the reply right moves with `exchange`
     if error use unit                          -- cut at 5 s: the host answers 503
   ```

6. **Strict parsing: refuse, never repair.**
   - **This record:** the host refuses every request that two HTTP implementations might read differently, with `400` and a closed connection: a bare LF or CR, empty lines before the request line, a folded header line, whitespace before a header's colon, a control character in a header value, both `Content-Length` and `Transfer-Encoding`, two `Content-Length` headers even when equal, a chunk extension, a `Transfer-Encoding` other than `chunked` alone, a `Connection` header that names anything but `close`, `keep-alive` and the connection's own headers, an `HTTP/1.0` request without `Host`, and, in the path, `.` and `..` segments and the encoded `/`, `\` and NUL (rules 10 and 11). These are the forms behind request smuggling, hop-by-hop header abuse and path confusion.
   - **Cost to the programmer:** none in code. A rare client gets `400`: one that sends bare LF, a blank line before the request, chunk extensions, an `HTTP/1.0` request without `Host`, or characters RFC 3986 does not allow unencoded, such as `|`, `{` and `"` in a query. HTTP allows a server most of this strictness (RFC 9112 §2.2, §3.2, §5.1, §5.2, §6.3). Two choices depart from it: RFC 9112 §2.2 asks a server to ignore at least one empty line before the request line, and §7.1.1 asks a recipient to ignore unknown chunk extensions. Both are refused because lenient handling of those places has been a route for smuggling, and clients rarely send them.
   - **Recommendation:** accept.
   - **Decided:** yes. The host refuses every ambiguous form and never repairs one (rules 10 and 11).

   ```text
   POST /orders HTTP/1.1          -- refused: 400 and the connection closes
   Host: shop.example
   Content-Length: 4
   Transfer-Encoding: chunked
   ```

7. **Secure defaults that cost configuration.**
   - **This record:** (i) without `hosts=`, a grant accepts only `localhost` and IP literals in `Host`, on every address, so DNS rebinding cannot reach any server that has not named its host names (rule 8); (ii) a finite default for connections from one peer, with IPv6 peers counted per /64 (rule 7); (iii) the bytes the host holds are budgeted per grant with a share per peer: request bodies (`buffers`, `peer_buffers`), unwritten replies (`replies`, `peer_replies`) and delivered requests (`delivered`), with a reserve per connection for its request head; a body that does not fit is refused with `503`, and under pressure a peer over its share of reply bytes loses its slowest connection, so slow peers cannot stall the others or end the run (rule 13).
   - **Cost to the programmer:** a server reached by name must list its names, `hosts=shop.example`, or opt out with `hosts=*`. A server behind a proxy or a load balancer, where every connection comes from the proxy's address, sets `peer=none`, `peer_buffers=none` and `peer_replies=none`, or larger values.
   - **Recommendation:** yes to all three.
   - **Decided:** yes to all three (rules 7, 8 and 13).

   ```toml
   grants = ["serve:http://0.0.0.0:8080;hosts=api.example;peer=none;peer_buffers=none;peer_replies=none"]   # behind a proxy
   ```

**For the project, not questions:**
- Every construct here widens what a program can do, so it can land while edition `0.4` is a preview only under ADR 0121's Question 1 (draft); if that record is not accepted, this one waits for `0.4` to become stable or for terms that allow it.
- The `ita.una` form of the grant lines and of `serve` (rules 7 and 16) is a proposal for ADR 0122 (draft, issue #893), which fixes their grammar. Under that record's assumption 8.1 (a), edition `0.4` reads only `ita.una`, so every `una.toml` example here becomes U7007 once it lands, with `una migrate` as the repair.
- The client of `spec/24` does not change. Its lenient framing cannot desynchronize it, since it sends `Connection: close` and reads one response per connection (`spec/24 §3` rule 2), and making it stricter would break working programs of editions `0.1` and `0.2`. Stricter client framing, if wanted, is future work for a later edition.

## Glossary

- **Label, join, `τ`, `τ⁰`, `τ_conc`, check point, stop point, interruptible operation, host stop request, hard stop.** As in ADR 0116's glossary.
- **Task, coroutine, worker, park, reactor, timing floor, token bucket, timing wheel.** As in ADR 0117's glossary.
- **Exchange.** One request and the right to answer it once: what `@server.next()` returns.
- **Reply right (`ReplyRight`).** The part of an exchange that names it to the host. It is a value, but not an authority: replying also needs `@server` with `reply`. It is not the ticket number with which ADR 0117 rule 8 orders the waiters of a limit.
- **Holding a reply right.** The task that may answer an exchange. Exactly one task holds each open reply right (rule 4).
- **Auto-answer.** The reply the host sends for a reply right whose holding task ended without answering.
- **Listener address.** An address and port that a grant lets the run accept connections on.
- **Serve thread.** A thread of the host that accepts connections, reads, runs TLS, parses and writes. In native code, the workers of ADR 0117 are the serve threads (rule 19).
- **Request smuggling.** Two HTTP implementations on one path, such as a proxy and a server, disagreeing about where a request ends, so that one request hides another.
- **Slowloris.** Holding many connections open by sending a request, or reading a reply, a few bytes at a time.
- **Origin-form.** A request target that is a path and an optional query, such as `/orders/7?full=1` (RFC 9112 §3.2.1).
- **DNS rebinding.** A web page that makes a browser send requests to a server on the browser's own machine, through a host name that the attacker points at `127.0.0.1`. A server defends itself by checking `Host`.

## Context

- **No server side.** `Http` is a client (`spec/24 §1`). No host capability accepts connections (ADR 0117, Context). The host capabilities are `Console`, `Clock`, `Env`, `Files`, `Keys` (`spec/05 §7`, `spec/14 §3`) and `Http` (`spec/24`).
- **Long-running code exists now.** `for ever` in a `diverges` function, stop points at iteration starts, `on stop`, interruptible operations that fail with `Interrupted` at a host stop, the grace period and the hard stop are accepted (ADR 0116 rules 8 to 18). The host-driven `[drive]` table hands a loop to the host (ADR 0116 rules 1 to 4). ADR 0116 rule 1 says that inputs other than ticks "come with the capability of #897".
- **Concurrency for servers exists now.** Tasks are coroutines on a few workers, pinned round-robin, and operations that wait park their task (ADR 0117 rules 1 to 3). `within`, `first of`, `tasks` and `start` are accepted (rules 5 to 7), with limits on `Http` grants and `limiter` (rules 8 and 9). ADR 0117 rule 3 lists "the operations that #897 marks" among those that park, and its implementation plan puts "the listener of #897" after the reactor (ADR 0117, Implementation plan, step 6).
- **The client's choices.** `Http` grants name a scheme, a host and a port (`spec/24 §2`). The client speaks HTTP/1.1 only, ignores proxy variables, and bounds every response (`spec/24 §3`, `§4`). Its arguments accept `internal`, and its responses and failures are `internal` (`spec/24 §5`). Bodies are `Text`, and a body that is not UTF-8 is `Protocol` (ADR 0074 H1). The client and its parser live in `crates/una-http` (`client.rs`, `parse.rs`), and URLs and grants in `crates/una-core/src/http.rs`, which both engines use (ADR 0074, amendment of 2026-10-07 "implementation decisions", item 4).
- **TLS.** `rustls` with the `ring` provider is in the trusted base (ADR 0074 H7; `Cargo.lock` pins `rustls` 0.23.46, `ring` 0.17.14, `rustls-pki-types` 1.15.1).
- **Labels of input.** Data from outside the program is `internal`: `Files.read`, `Env.get` and every `Http` response (`spec/14 §3`, `spec/24 §5`). `Env.get` returns `Option[internal Text]`, so whether a variable exists is `public` while its value is `internal` (`spec/14 §3`, table).
- **Second-class authority.** Capability references are not values (`spec/05 §6`). Handlers have no mutable state: their parameters are captured values (`spec/05 §5` rule 3).
- **No bytes.** Una has no type of bytes and no streams; `Text` is UTF-8 (`spec/22 §1`, ADR 0074 H1, H4).

Issue #897 asks for a host `HttpServer` capability, and possibly a `Listener`, granted by address and port with the limits of #896; request and reply types with limits and labels; how a `for ever` loop or a host-driven step receives requests; TLS, timeouts, backpressure and graceful shutdown; information flow for responses; both engines and the cost per request.

### Departures from the issue

1. **No `Listener`** in this record (Question 1).
2. **Requests come through `next()`, not `accept()`.** ADRs 0116 and 0117 sketch `@listener.accept()` returning a connection. With HTTP keep-alive, one connection carries many requests, and the unit a program answers is a request. The host keeps connections to itself, so no connection is ever a value (Appendix A).
3. **The host-driven form is `[serve]`, not `[drive]`.** A step function with a state answers one input at a time. A server answers many at once, so `handle` has no state and runs in its own task per request (rule 16).

## Decision

In one paragraph: in edition `0.4`, a module may import from `std.host` a capability `HttpServer` with three operations, `next`, `reply` and `close` (rules 1 to 5). A grant `serve:<scheme>://<address>:<port>` lets the run accept connections there, with limits on connections, sizes, time, concurrency and rate, a `Host` check, and TLS keys that never enter the program (rules 6 to 9). The host speaks a strict HTTP/1.1 that refuses every ambiguous form (rules 10 to 14). A server is a `[serve]` module with `handle(request) -> Reply`, or a `for ever` loop with `tasks` and `start` (rules 15 to 17). A host stop closes the listeners, drains the requests in flight and runs `on stop` (rule 18). Requests are `internal`, replies accept `internal`, and no deadline of the host becomes a value of the program (Information flow).

### A. Edition and names

0. **Edition and names.**
   - **Edition.** Every construct of this record exists only in packages of edition `0.4` (ADR 0116 rule 0). In earlier editions, an import of these names from `std.host` is U1001, as an unknown name is (`spec/10 §11` rule 4); a `serve:` grant is a usage error, or U7002 in `una.toml`; and `[serve]` is an unknown table (U7002).
   - **No new keywords.** This record adds no reserved or contextual keyword.
   - **Imported, not in the prelude.** `std.host` declares the names of rule 1, but the prelude does not put them in scope: a module imports them, as `import std.host (HttpServer, Request, Reply)`. `spec/10 §11` rule 10 gains them, with the note that an import of them is not redundant. So a user kind named `Request` or `Reply` stays legal in every edition, and edition `0.4` reserves no name for this record.

### B. The capability

1. **Declarations.** `std.host` declares, in edition `0.4`:

   ```una
   --- The methods a request may have. The host answers any other with 501.
   pub kind Method = Get | Head | Post | Put | Patch | Delete | Options

   --- One decoded query parameter, as `name=value`.
   pub kind Param
     name: internal Text
     value: internal Text

   --- A request, as the host read it (rules 10 and 11).
   pub kind Request
     method: internal Method
     target: internal Text             -- the request target as received, in origin-form
     segments: internal List[Text]     -- the path, split on "/" and percent-decoded segment by segment
     query: internal List[Param]       -- the query, decoded, in order
     headers: internal List[Header]    -- names in ASCII lower case, in the order received
     body: internal Text
     peer: internal Text               -- the peer's IP address, without its port
     listener: internal Text           -- the grant's address, as the grant writes it

   --- A request and the reply right to answer it.
   pub kind Exchange
     request: internal Request
     right: internal ReplyRight

   pub kind Status = Int where 200 <= it and it <= 599

   pub kind Reply
     status: internal Status
     headers: internal List[Header]
     body: internal Text

   pub kind ServeError = Interrupted | TimedOut | Closed | NotHeld | BadReply

   pub capability HttpServer
     op next() -> internal Exchange
       fails ServeError
     op reply(to: ReplyRight, reply: Reply) -> Unit
       fails ServeError
       accepts internal
     op close() -> Unit
   ```

   - **`Header`** is the kind of `spec/24 §1`, whose `value` is `internal Text`.
   - **Labeled fields.** The fields of these kinds are declared `internal`, as `Header.value` is, so a program can build a `Reply` from request data, and a test or a handler can build a `Request` from `internal` data. A constructor field is a flow site (`spec/07 §7` rule 2), so a `secret` value in a field of a `Reply` is U4001 where the reply is made.
   - **`ReplyRight`** is an opaque type of `std.host`. It has no constructor and no literal. Its values come only from `next` and from `std.testing.test_exchange` (rule 21). Two reply rights are equal when they name the same exchange. Interpolation writes `<reply-right>`.
   - **Failures.** `next` fails with `Interrupted` at a host stop (ADR 0116 rule 14), `TimedOut` when a deadline of the program cuts its wait (ADR 0116 rule 11, ADR 0117 rule 5), and `Closed` after `close`. `reply` fails with `NotHeld` (rule 4) and `BadReply` (rule 13). `close` cannot fail.
   - **What parks.** `next` parks its task while no request is ready, and it is an interruptible operation (ADR 0116 rule 12, ADR 0117 rules 3 and 4). `reply` and `close` never wait: `reply` hands the reply to the host and returns (rule 13).
   - **`close`** stops accepting connections: the host closes the listeners, answers the requests read but not yet delivered with `503` and `Connection: close`, lets each connection finish the exchange it has open, and closes it. Later calls of `next` fail with `Closed`. A second `close` does nothing. It accepts `public` only, so a server cannot close itself on the strength of a request's content (Information flow, IF1).
   - **One instance.** A run has one host instance of `HttpServer`, `host:HttpServer`, which serves every `serve:` grant. Its `next` returns the requests of all of them, in the order the host finished reading them on each worker (rule 19).
   - **Trace events.** `next` and `reply` are events of `spec/11 §5`, with `instance` `host:HttpServer`. Their values are redacted by their labels, as every event's are, and, whatever the label and also in any recording kept for replay, the host writes them in this reduced form:
     - the values of the headers `authorization`, `proxy-authorization`, `cookie`, `set-cookie`, `x-api-key` and `x-auth-token` are `<credential>`, in requests and in replies;
     - the values of query parameters, in `query` and in `target`, are written as bodies are, below;
     - a body is `{"length": n, "hmac": "<hex>"}`: its length in bytes and an HMAC-SHA-256 of it under a key of the run, drawn at random when the run starts and all zeros under `--serve-script`, so that two runs of a script compare while a guessable body cannot be found from its trace;
     - a reply right is `<reply-right>`.

     **What stays in clear:** path segments, other header values, and statuses. A credential that a client puts in a path, or in a header of another name, is written as it came; a server that accepts such credentials keeps traces where only its operators can read them. **Replay.** `spec/11 §5` keeps host results in full for a future `una replay`; a server's recording does not keep bodies or credentials, so a server is replayed from its `--serve-script` input (rule 21), not from its recording.
   - **Examples.** An example that uses `HttpServer` without a `given` handler is U3014, as for every host capability (`spec/05 §9` rule 5). The repair of U3012 for `HttpServer` is a description only, "grant `serve:<scheme>://<address>:<port>`", since no address can be chosen for the programmer.

   ```una
   let next = attempt @server.next()
   when next
     is Ok(exchange) ->
       @server.reply(to: exchange.right, reply: Reply(status: 204, headers: [], body: ""))
         if error use unit
     is Err(failure) -> unit
   ```
2. **What the program sees of a request.** The host has already:
   - checked the framing, the sizes, the `Host` header and the target (rules 10 and 11);
   - read the whole body, at most the grant's `body` limit, and checked that it is UTF-8;
   - removed the headers that belong to the connection, not to the request: `Connection`, `Keep-Alive`, `Proxy-Connection`, `TE`, `Trailer`, `Transfer-Encoding`, `Upgrade` and `Content-Length`. `Connection` may list only `close`, `keep-alive` and those names; any other token in it is `400` (rule 10), so a peer can never make the host delete a header that a front proxy added, or `host`;
   - answered `Expect: 100-continue` itself (rule 10).

   So `headers` holds the request's own headers, `host` among them, and `body` the whole body. `peer` is the address of the connection's other end, with an IPv4-mapped IPv6 address written as IPv4; it is a proxy's address when the server stands behind one, and a forwarded address in a header is the program's to trust or not. For an absolute-form request, `target` is the origin-form the host rewrote it to (rule 11).
3. **Replies.**
   - **`status`** is between 200 and 599, a refinement of `Status` that the verifier checks where a `Reply` is made (`spec/12 §3`). A `1xx` reply is the host's alone, and `101` would switch protocols, which this record does not allow.
   - **No body with `204` and `304`.** `reply` fails with `BadReply` when one has a body (rule 13).
   - **Headers.** The program sets any header except those the host writes or that belong to the connection (rule 13).
   - **`HEAD`.** A reply to a `Head` request is sent without its body, with the `Content-Length` of the body the program gave.

   ```una
   Reply(status: 201, headers: [Header(name: "location", value: "/orders/7")], body: "")
   ```
4. **Reply rights and who holds them.**
   1. **`next`** gives the reply right of its exchange to the task that called it.
   2. **`start e`** moves to the started task every reply right that the calling task holds and that is reachable from the values `e` captures, closures included, in the order the runtime copies them (ADR 0113, ADR 0117 rule 7). The calling task no longer holds them.
   3. **No other construct moves a reply right.** A `together` branch, a `together for` body and a `first of` branch act for the task that runs the construct, and none of them may perform an operation of `HttpServer` (rule 5).
   4. **`reply`** succeeds only for a reply right that the calling task holds and has not answered. Otherwise it fails with `NotHeld`, and nothing is sent. A successful `reply` releases the reply right.
   5. **The end of a task.** When a task ends while it holds reply rights, by returning, by a failure, by ending alone past its depth (ADR 0117 rule 7), by a cancellation or at the hard stop, the host sends an auto-answer for each one, in the order the task received them: `500 Internal Server Error` when the task returned, failed or ended alone; `503 Service Unavailable` with `Retry-After: 1` when it was cancelled, timed out by `within`, or stopped. Each appends the event `{"seq": n, "task": "<path>", "outcome": {"kind": "unanswered", "status": s}}` in the task.
   6. **Determinism.** Which task holds a reply right depends only on the order of `next`, `start` and `reply` in each task, never on timing. So `NotHeld` is a function of the program's own data and order.
   7. **One state per exchange.** On the host's side, each exchange is open until one atomic transition ends it, by the program's `reply`, by the host's `answer` deadline, by an auto-answer, by its peer leaving, or by a stop. The first transition wins, and the others send nothing. The program's view of rules 4.1 to 4.6 does not change with the order: a `reply` after another transition returns `unit` (rule 14), and an auto-answer after one still appends its `unanswered` event, so the trace does not depend on timing. Delivery by `next` and its interruption are one atomic step too: an exchange is either delivered, or left in the queue when `next` fails, never taken off the queue and lost.

   ```una
   tasks
     for ever
       let next = attempt @server.next()       -- this task now holds the reply right
       when next
         is Ok(exchange) ->
           start answer(exchange)               -- the started task holds it
         is Err(failure) -> unit
   ```

   A task's reply rights are found by the walk that copies its captures, so the walk costs nothing in native code, and in the interpreter only over captured values whose type can hold a `ReplyRight` or a function (Costs).
5. **No server operations in branches** (U3033). The effect set (`spec/05 §3` rule 3) of a `together` branch, of the body of a `together for`, and of a branch of `first of` MUST NOT contain an operation of `HttpServer`. Those tasks run at once and act for one task, so two of them could answer one reply right in an order that timing decides. Work that needs concurrency inside one request runs in branches that compute, and the reply follows the construct.

   ```una
   together
     let a = price(order)                         -- fine
     let b = @server.reply(to: t, reply: done)    -- U3033: a server operation in a branch
   ```

### C. Grants

6. **Syntax.** In `[run] grants` and `--grant` (`spec/13 §5`): `serve:<scheme>://<address>:<port>`, followed by limits and options as suffixes `;<item>`, in the form of ADR 0117 rule 8.
   - `<scheme>` is `http` or `https`.
   - `<address>` is an IPv4 address, or an IPv6 address in brackets, such as `[::1]`. `0.0.0.0` and `[::]` mean every interface. Host names are refused: a grant names where the run listens, and nothing resolves it.
   - `<port>` is from 1 to 65535 and is always written.
   - **No overlap.** Each address and port may be granted once, and a wildcard grant (`0.0.0.0` or `[::]`) may not share its port with another grant of the same family (U7002, or a usage error for `--grant`). IPv6 sockets are bound with `IPV6_V6ONLY` set, so `[::]:p` and `0.0.0.0:p` are two grants that do not overlap.
   - **Paths.** The paths of `certificate` and `key` cannot contain `;`, which ends an item.

   ```toml
   [run]
   entry = "shop.main"
   grants = ["serve:http://127.0.0.1:8080;64/once;600/minute;body=256"]
   ```

   - **Errors.** U7002 in `una.toml`, and a usage error, exit code 2, for `--grant`: a malformed spec; an item that rule 7 does not list or that is given twice; a value out of range; `certificate`, `key` or `tls12` on an `http` grant, or `certificate` or `key` missing on an `https` one.
   - **Checks.** A `uses` of an `HttpServer` operation that no `serve:` grant allows is U3012, as for every host capability (`spec/05 §7` rule 2). A grant allows all three operations.
7. **Limits and options.** Each item has a default, so every server is bounded whether its grant says so or not:

   | Item | Meaning | Default | Range |
   | --- | --- | --- | --- |
   | `N/once` | exchanges delivered by `next` and not yet answered | 256 | 1 to 2^31 − 1 |
   | `N/second`, `N/minute`, `N/hour` | exchanges delivered per period, a token bucket (ADR 0117 rule 8) | none | 1 to 2^31 − 1 |
   | `waiting=N` | requests read and waiting to be delivered | 1024 | 0 to 2^31 − 1 |
   | `connections=N` | open connections | 1024 | 1 to 1,048,576 |
   | `peer=N` | open connections from one peer (rule 12) | `max(16, connections / 16)` | 1 to 1,048,576, or `none` |
   | `handshakes=N` | TLS handshakes started per second; one peer may start `max(4, N / 16)` | 512 | 1 to 2^31 − 1 |
   | `requests=N` | requests on one connection | 1000 | 1 to 2^31 − 1 |
   | `line=K` | the request line, in KiB | 8 | 1 to 64 |
   | `headers=N` | header lines of a request, trailers included | 100 | 1 to 1000 |
   | `header=K` | the header section of a request, trailers included, in KiB | 16 | 1 to 256 |
   | `body=K` | a request's body, in KiB | 1024 | 0 to 1,048,576 |
   | `buffers=M` | bytes of request bodies the host holds for this grant, being read or waiting for delivery, in MiB (rule 13) | 64 | 1 to 65,536 |
   | `peer_buffers=M` | the share of `buffers` that one peer may hold, in MiB | `max(1, buffers / 64)` | 1 to 65,536, or `none` |
   | `replies=M` | bytes of replies handed to the host and not yet written, in MiB (rule 13) | 64 | 1 to 65,536 |
   | `peer_replies=M` | the share of `replies` that one peer may hold under pressure, in MiB | `max(1, replies / 64)` | 1 to 65,536, or `none` |
   | `delivered=M` | bytes of the requests of open exchanges, delivered by `next` and not yet answered, in MiB (rule 13) | 256 | 1 to 65,536 |
   | `idle=S` | seconds a connection may wait for the first byte of a request | 5 | 1 to 3600 |
   | `read=S` | seconds to read a whole request from its first byte; for a connection's first request, from the accept, the TLS handshake included | 20 | 1 to 3600 |
   | `write=S` | seconds to write a whole reply | 30 | 1 to 3600 |
   | `answer=S` | seconds from the end of reading a request to the start of its reply | 30 | 1 to 86,400 |
   | `hosts=a,b` | the host names `Host` may carry (rule 8) | IP literals and `localhost` | names, a leftmost `*` as in `spec/24 §2`, or `*` |
   | `certificate=F`, `key=F` | the certificate chain and the private key, PEM files (rule 9) | none | `https` only, and required there |
   | `tls12` | also accept TLS 1.2 (rule 9) | off | `https` only |

   - **Settled.** Every default here is settled when the construct lands: these defaults bound denial of service, and ADR 0121 rules 5 and 17 (draft) keep security properties out of unsettled points. A later release may only tighten a default, as a stricter change. This record declares no unsettled point.
   - **Arithmetic.** Sizes and counts are compared in unsigned 64-bit arithmetic, with checked or saturating operations, and never wrap; every target of the runtime is a 64-bit target (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, as ADR 0117 rule 2 lists them). A token interval of a rate is rounded up, never to zero: `N/second` with N above 10^9 has an interval of 1 ns. The check of the run's descriptor limit (Diagnostics) sums `connections` over every grant.
   - **`ita.una`.** ADR 0122 (draft) writes grants one per line under `run grants`, with ADR 0117's limits as lines below them. This record proposes, for `serve`, the same items with the same names and units:

     ```una
     serve "https://0.0.0.0:8443"
       certificate "tls/chain.pem"
       key "tls/key.pem"
       at most 256 at once
       at most 6000 per minute
       waiting at most 1000
       connections at most 4096
       peer at most 64
       body at most 1024 KiB
       answer within 10 seconds
       hosts "shop.example", "*.shop.example"
     ```

     ADR 0122's `grant` production then gains `"serve" text_lit` with these lines, its `section` production gains `serve` (rule 16), and its rule 12 reads "a dependency's `run`, `drive` and `serve` sections are not read".
   - **Enforcement.** Limits sit in the host, below every handler and rebinding, as ADR 0117 rule 8 places those of `Http`. Which state is shared between workers, and which order is kept, is in rule 19.
   - **Global limits are levers too.** A limit per grant can be spent by a few peers: 16 peers at their share of `handshakes` exhaust the grant's rate and close everyone's new connections for that second, and 64 peers at `peer_buffers` fill `buffers`, after which bodies get `503`. The shares make this cost many addresses, not one; they do not remove it. An operator facing it raises the grant's limits or puts a filter in front.
8. **The `Host` header.** A request MUST carry exactly one `Host`, as RFC 9112 §3.2 requires, `HTTP/1.0` requests included, else `400`.
   - **Syntax, on every grant.** Its value is a `uri-host` and an optional `:` with a port of at most five digits (RFC 9110 §7.2, RFC 3986 §3.2.2): a registered name of letters, digits, `-`, `_` and `.`, an IPv4 address, or an IPv6 address in brackets. One trailing `.` of a registered name is removed before any comparison, so `shop.example.` matches `shop.example`; the program sees the value as sent. Anything else, such as `a/b@c`, is `400`, so programs that build absolute URLs from `Host`, for a link in a password-reset mail, never see a malformed one.
   - **With `hosts=`,** its name, compared without its port and in ASCII lower case, MUST match one of the names, else `421 Misdirected Request`. `hosts=*` accepts every name of valid syntax.
   - **Without `hosts=`,** on every grant, whatever its address, `Host` MUST be `localhost` or an IP literal, else `421`. DNS rebinding needs a DNS name, so no web page can reach a server that has not named its host names, whether it listens on `127.0.0.1`, on `0.0.0.0` inside a container, or on a private address. An IPv4-mapped IPv6 address, such as `[::ffff:127.0.0.1]`, is read as the IPv4 address it maps.
   - **Over TLS,** a server name the client sent in the handshake MUST equal the `Host` name, else `421`.
9. **TLS.**
   - **Loading.** Before `main` starts, the host reads `certificate` (a PEM chain, leaf first) and `key` (a PEM private key in PKCS#8, SEC1 or PKCS#1 form), with the PEM reader of `rustls-pki-types`, and checks that the key matches the certificate. Relative paths resolve as the paths of other grants do (`spec/13 §5`). A file that cannot be read, a malformed one, and a key that does not match are usage errors, exit code 2, before anything runs.
   - **The key file, without a race.** The host opens the key file once, following symbolic links, since secret volumes are links (Question 3), and then checks the open descriptor with `fstat`, never the path: it MUST be a regular file, owned by the effective user of the run or by root, with no write bit for its group and no bit for others (`st_mode & 0o027 == 0`); its group read bit MAY be set only when the file's group is the run's effective group or one of its supplementary groups (`getgroups`), which is how a non-root pod reads a secret mounted with `fsGroup` and mode `0440`. It then reads the key from that same descriptor. A file swapped between a check and a read is therefore impossible, and a key owned by another user, whose content that user controls, is refused even when the run is root's. Otherwise the run does not start, a usage error. On Windows, an `https` grant is a usage error until a later record defines the check of the file's access list.
   - **The key stays in the host.** No operation returns it. It is in no value, trace, diagnostic or report.
   - **Protocol, pinned.** TLS 1.3 only, unless the grant says `tls12`, which adds TLS 1.2 with the extended master secret required (`require_ems`). The cipher suites of `rustls` with the `ring` provider. No renegotiation, which `rustls` does not implement. No early data (`max_early_data_size` 0) and no half-RTT data, since a replayed early request could repeat an effect. Resumption only through a bounded in-memory session cache; stateless tickets are off. The key log is the one that writes nothing, so `SSLKEYLOGFILE` is never read. ALPN offers `http/1.1` only. No client certificates. A handshake that fails closes the connection without an HTTP reply.
   - **Handshake cost.** Handshakes are bounded per second by `handshakes`, per grant and per peer, with token buckets, O(1) each; a connection over either rate is closed at its accept, before any handshake work. At most half of the workers, rounded up, run handshakes at a time, so a flood of handshakes leaves the other half to requests (rule 19).
   - **No new crate.** `rustls` serves the server side as it serves the client's. If its PEM reader needs one more feature of `rustls-pki-types`, that feature is enabled; no crate joins `Cargo.lock`.

### D. HTTP/1.1 on the wire

10. **Reading a request.** The host reads each request by RFC 9112 with these choices. Each refusal sends the status shown, with `Connection: close`, and closes the connection after it, so the host never tries to find where a broken request ends.
    - **Request line.** `method SP target SP version CRLF`, single spaces, with no empty line before it (`400`, Question 6). A method that is not a token is `400`; a token that is not a method of `Method` in upper case is `501`, `CONNECT` and `TRACE` included. A version `HTTP/1.1` or `HTTP/1.0`, else `505`. A line longer than `line`, `414`.
    - **Line ends.** Every line ends with CRLF. A bare LF, or a CR not followed by LF, anywhere in the request line, the headers or the chunked framing, is `400`.
    - **Header lines.** A name is a token (RFC 9110 §5.6.2) followed at once by `:`; whitespace before the colon is `400`. A line that starts with whitespace (obsolete folding) is `400`. A value is trimmed of spaces and tabs at both ends; a byte below 0x20 other than tab, or 0x7F, in it, or a value that is not UTF-8, is `400`. A `Connection` header whose tokens, compared in any letter case (`Keep-Alive` is `keep-alive`), are not all among `close`, `keep-alive` and the connection's own headers of rule 2 is `400`. More than `headers` lines, or a section larger than `header`, is `431`.
    - **Framing, the defense against smuggling.**
      - `Transfer-Encoding` and `Content-Length` together: `400`.
      - `Transfer-Encoding` other than one header whose whole value is `chunked`, in any letter case: `501` when it names a coding, `400` when it is malformed or given twice, `chunked` twice included.
      - `Transfer-Encoding` in an `HTTP/1.0` request: `400`.
      - `Content-Length`: one header, whose value is ASCII digits only, at most 19 of them: else `400`. Two headers, even with equal values: `400`. A value above `body`: `413`, without reading the body.
      - Chunked bodies: each size is hex digits alone, with no whitespace, sign or extension, at most 16 digits; anything else is `400`. A body that grows past `body` is `413`. Trailers are read and dropped, and their lines and bytes count toward the same `headers` and `header` limits as the header section, not a fresh allowance.
      - Neither header: an empty body.
    - **Encodings.** A `Content-Encoding` other than `identity` is `415`: the host never decompresses, so no small request expands into a large one.
    - **`Expect`.** In an `HTTP/1.1` request, `100-continue`, in any letter case, makes the host send `100 Continue` once it has admitted the body (rule 13): the declared `Content-Length`, or, for a chunked body, the first reservation of rule 13; any other expectation is `417`. In an `HTTP/1.0` request, `Expect` is ignored, as RFC 9110 §10.1.1 says.
    - **UTF-8.** A body that is not valid UTF-8 is `400` (Question 1).
    - **Splits.** Reading is incremental, and a request split at any byte boundary reads exactly as in one piece.
    - **The server's own framing code.** The client's functions are lenient where this rule is strict (`crates/una-http/src/parse.rs`): `chunk_size` drops an extension after `;` and allows at most 15 hex digits; the reading of the head accepts two equal `Content-Length` values; `header` repairs a value that is not UTF-8 with `String::from_utf8_lossy`; and the trailers' byte count starts again at 0. The client keeps them: it sends `Connection: close` and reads one response per connection (`spec/24 §3` rule 2), so its leniency cannot desynchronize it, and editions `0.1` and `0.2` keep their behavior. The server has its own strict functions beside them: an extension is an error, a size has 1 to 16 hex digits and fits in 64 bits, two `Content-Length` headers are an error, a value that is not UTF-8 is an error and is never repaired, and trailers count toward the limits of the head. Differential tests run both readers over one corpus and require the same result on every input that both accept.
    - **Closing after a refusal.** After a `400`, `413`, `431` or `503` that closes the connection, the host shuts down its sending side, reads and drops what the peer still sends, at most 64 KiB or one second, and then closes, so that the peer reads the status instead of a reset.
11. **The target.**
    - **Forms.** Origin-form only: a target that starts with `/`. Absolute-form (`http://host/path`) is accepted only when its scheme is the grant's and its authority equals `Host`, and is then rewritten to the origin-form of its path and query, which is what `target` holds. The asterisk-form and the authority-form are `400`. A `#` is `400`.
    - **Characters.** The path and the query hold only the characters RFC 3986 allows there, and each `%` is followed by two hex digits; else `400`.
    - **Segments.** The path after its first `/` is split on `/`, and each segment is percent-decoded alone, so an encoded `/` could never make a new segment. Even so, a segment that decodes to `.` or `..`, or that holds `/`, `\`, NUL or another control character, or that is not UTF-8 after decoding, is `400`. `/` gives `[""]`, and `/a//b` gives `["a", "", "b"]`.
    - **Query.** Split on `&`; each part is `name=value` or `name` (value `""`); `+` decodes to a space, then percent-decoding; empty parts are skipped. A name or value that is not UTF-8 after decoding is `400`.
12. **Connections.**
    - **One request at a time.** The host reads a connection's next request only after it has written the reply to the current one, so replies leave in the order of requests, and a pipelined client is served in order.
    - **Keep-alive.** An `HTTP/1.1` connection stays open after a reply unless the request said `Connection: close`, the connection reached `requests`, a refusal of rule 10 happened, or a stop or `close` is under way. An `HTTP/1.0` connection closes after one reply.
    - **Time.** Each connection has at most one timer at a time: `idle` while it waits for a request, `read` while it reads one, `write` while it writes a reply. A request that passes `read` is answered `408` and the connection closes; a connection idle past `idle`, or a reply not written within `write`, is closed. These are totals, not per-read timeouts, so trickling bytes cannot stretch them (as in `spec/24 §4` rule 2). They bound how long one slow connection is held, not how many an attacker holds: that is the work of `peer` and `connections`.
    - **Peers.** A peer is an IPv4 address, or the /64 prefix of an IPv6 address, since one host commonly holds a whole /64; an IPv4-mapped IPv6 address counts as its IPv4 address. The host keeps a table of the peers with open connections, at most `connections` entries, O(1) per accept and close. Counting per /64 can still be evaded from a /48, which holds 65,536 /64s and which sites commonly get; that is out of model, and the totals of `connections`, `buffers` and `replies` still bound such an attacker.
    - **Capacity.** At `connections` open connections the host stops accepting, and the kernel's backlog holds the rest. A new connection from a peer that already has `peer` open connections is closed at once.
13. **Writing a reply.**
    - **Checks at `reply`.** A `204` or `304` reply has an empty body. Each header name is a token, in any case, and each value holds no CR, LF, NUL or other control character but tab. A reply sets none of `Connection`, `Content-Length`, `Date`, `Keep-Alive`, `Proxy-Connection`, `TE`, `Trailer`, `Transfer-Encoding` and `Upgrade`, in any letter case. It has at most 100 headers and 64 KiB of them, as a response the client reads may (`spec/24 §4` rule 1). Otherwise `reply` fails with `BadReply` and nothing is sent; the reply right stays held, so the program may still answer.
    - **The bytes.** `HTTP/1.1 <status> <reason>`, with the reason phrase of RFC 9110 §15 or an empty one; then `Date`, `Content-Length` except for a `204` or a `304` (RFC 9110 §8.6 forbids it on a `204`, and on a `304` it would misstate the representation), the program's headers in order, and `Connection: close` when the connection will close; then the body, unless the request was `HEAD` or the status is `204` or `304`.
    - **Default type.** A reply without `content-type` gets `content-type: text/plain; charset=utf-8`, since every body is UTF-8 `Text`, so no client guesses a type for a reflected body. A `204`, a `304` and a reply to `HEAD` get none, since they carry no body.
    - **Hand-off.** `reply` serializes the reply into one buffer of the host and returns; the worker that owns the connection writes it. While a connection's reply is being written, the host reads nothing more from that connection only.
    - **Bytes of replies, in shares.** The bytes of replies handed to the host and not yet written are charged to the grant's `replies` and to the peer's share, `peer_replies`, never to `--max-memory`: the host reserves `replies` for each grant at start, outside the run's budget, so charging a reply never raises U6003, and `reply` never waits or fails for it. When the grant is over `replies`, the host closes connections until it is under again: first the slowest connection, the one whose reply has the most bytes left, of the peer furthest over its share; when no peer is over its share, the slowest connection of all. A closed connection's reply is discarded, which the program never sees (rule 14), so IF4 holds. A single reply larger than `replies` is written only while the grant has no other reply pending, else its connection is closed at once.
    - **Bytes of requests, in shares.** The host holds the bytes of requests in three reserves, so that slow peers can stall only themselves:
      1. **Heads.** Each open connection has its own reserve for one request line and one header section, `line` + `header`, counted in `connections` at its accept. The reserve is accounting, not allocation: the bytes are allocated as they arrive and released when the connection is idle. Reading a head never waits for the shared budget.
      2. **Bodies.** A body is admitted before it is read: a declared `Content-Length` reserves its whole length; a chunked body reserves 64 KiB at a time, before each step. A reservation must fit the peer's share, `peer_buffers`, and what remains of the grant's `buffers`. When it does not fit, the host answers `503` with `Retry-After: 1` at once and closes the connection (rule 10), without stalling anything: a full budget never stops the reading of a request that has its reservation, or of any head. The reservation is released when the request is delivered by `next`, or refused.
      3. **Delivered requests** live in the program's heap, and count toward `--max-memory` as every value does (ADR 0117 rule 2). Their bytes, head and body, are also charged to the grant's `delivered` budget from delivery until the exchange ends; delivery by `next` waits until the next request fits, and a request that waits past `answer` is refused with `503`. The budget depends only on the requests and on how fast the program answers them, not on the program's other memory, so a server whose own state fills most of `--max-memory` still serves.
    - **The worst case, stated.** With the defaults, one grant holds at most: heads, 1024 connections × 24 KiB = 24 MiB; bodies being read or waiting, 64 MiB, of which one peer holds at most 8 MiB; delivered requests, at most `delivered` = 256 MiB of bytes in the heap, and about twice that as Una values (a `Text` with its index, the cells of `headers`, `segments` and `query`); unwritten replies, at most `replies` = 64 MiB, outside `--max-memory`. Rule 7's items scale each term; with the default `--max-memory` of 4096 MiB (`spec/11 §7`), the delivered requests of every grant fit with room for the program's own state.
14. **The host's own answers.** Besides the refusals of rules 8, 10 and 11, the host answers:
    - `503` with `Retry-After: 1`, when a request finds `waiting` requests already queued, when its body does not fit its shares (rule 13), when it waits longer than `answer` for delivery, a `/once` slot or room in `delivered`, when `close` or a host stop finds it undelivered or still being read, and when the holding task is cancelled (rule 4.5);
    - `503` when `answer` passes after delivery without a reply. A `reply` that comes later is discarded: it returns `unit`, its trace event shows outcome `ok`, and nothing is sent;
    - `500` when the holding task ends without answering (rule 4.5).

    When a peer closes its connection before its reply is written, the reply is discarded in the same way. So no outcome of `reply` depends on the host's deadlines or on a peer's patience (Information flow, IF4).

### E. Writing a server

15. **The language form.** A server is a `diverges` function with a `for ever` loop that calls `next`, and starts a task per request:

    ```una
    serve_all() -> Unit
      uses @server: HttpServer[next, reply, close]
      diverges
    =
      tasks at most 1000 at once
        for ever
          let next = attempt @server.next()
          when next
            is Ok(exchange) ->
              start answer(exchange) within 10 * second
                if error use unit
            is Err(failure) -> unit
        on stop
          @server.close()
    ```

    - `answer` is the program's own function, for example:

      ```una
      answer(exchange: internal Exchange) -> Unit
        uses @server: HttpServer[reply]
      =
        @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "ok"))
          if error use unit
      ```
    - `next` waits, parked, and fails with `Interrupted` at a host stop, which ends the loop at its next iteration start (ADR 0116 rule 14).
    - `start ... within` bounds the time of each request in code; the host's `answer` bounds what the peer waits.
    - `tasks at most N at once` bounds the program's tasks, and the grant's `/once` bounds the exchanges open, so either is a limit on work in flight.
    - A server that answers in its loop, with no `tasks`, is legal too: it answers one request at a time.
16. **The host-driven form: `[serve]`.**
    - **The table.** In edition `0.4`, `una.toml` MAY hold `[serve]` with `module`, a module of the package, and optionally `within`, a per-request deadline in milliseconds from 1 to 86,400,000. `[serve]` together with `[run] entry` or `[drive]`, a missing `module`, a value out of range, or `[serve]` in an earlier edition, is U7002. `[run] grants` keeps its meaning, and a run with `[serve]` and no `serve:` grant is a usage error, exit code 2.
    - **`ita.una`.** When it replaces `una.toml` (ADR 0122, draft), its `serve` section has the same keys, as its `drive` section has those of `[drive]` (ADR 0116 rule 1): `serve = "serve" NEWLINE INDENT "module" module_path NEWLINE [ "within" int_lit NEWLINE ] DEDENT ;`.
    - **The module** declares `pub handle(request: internal Request) -> internal Reply`, and optionally `pub on_stop() -> Unit`. Both MAY have `uses`, checked against the grants as `main`'s are (U3012). `handle` MAY declare `fails`. `on_stop` MUST NOT (U3029). Neither may be `diverges` (U3027). A module without `handle` of this shape is a usage error, exit code 2, as for `[drive]` (ADR 0116 rule 2).
    - **Meaning.** `una run` and the executable that `una build` writes run this program, with `handle` and `on_stop` from the module:

      ```una
      main() -> Unit
        uses @server: HttpServer[next, reply]
        diverges
      =
        tasks
          for ever
            let next = attempt @server.next()
            when next
              is Ok(exchange) ->
                start answer(exchange) within w   -- `within w`, and the clause below, only when [serve] within is set
                  if error use unit
              is Err(failure) -> unit
        on_stop()                                 -- only when declared, after every request has drained

      answer(exchange: internal Exchange) -> Unit
        uses @server: HttpServer[reply]           -- and the `uses` of handle
      =
        let reply = handle(exchange.request)
          if error use Reply(status: 500, headers: [], body: "")   -- only when handle declares fails
        @server.reply(to: exchange.right, reply: reply)
          if error use unit
      ```

      A failure of `handle` becomes `500` with an empty body, never the failure's payload, and its trace event stays in the request's task.
    - **Why no `within` by default.** A `within` puts `handle` under the timing floor of ADR 0117 IF2, so a `handle` whose timing depends on `secret` data could not reply under one. `handle` cannot be `diverges`, so every request ends after finitely many steps (ADR 0116 rule 9), and its waits on `Http` end within `--http-timeout`; the host's `answer` already bounds what the peer waits.
    - **Testing.** `handle` is an ordinary function, so tests call it directly:

      ```una
      example "the health endpoint answers ok"
        do handle(test_request(method: Get, target: "/health", body: ""))
        expect result.status == 200
      ```
17. **Backpressure, from the kernel to the program.** Work is refused as early as it can be, and every queue is bounded:
    1. the kernel's backlog, while the host is at `connections`; a new connection from a peer already at `peer` is closed at once (rule 12), not held;
    2. a body that does not fit its reservation in `peer_buffers` or `buffers` is refused with `503` at once, and nothing stalls (rule 13); a reply over `replies` closes the slowest connection of the peer furthest over `peer_replies` (rule 13);
    3. the queue of requests read and not delivered, `waiting`, after which the host answers `503`;
    4. delivery, which waits for a `/once` slot, a token of the grant's rate and room in `delivered`;
    5. in the program, `tasks at most N at once`, where `start` parks at its bound (ADR 0117 rule 7), and `limiter` (ADR 0117 rule 9).

    A request waits in at most one of 3 and 4 at a time, and every wait is bounded by `answer`.

### F. Stopping

18. **Graceful shutdown.** At a host stop request (ADR 0116 rule 14):
    1. the host closes every listener at once, and closes idle keep-alive connections;
    2. requests read but not delivered get `503` with `Connection: close`, and so do requests still being read, as soon as their head is read, or at once when their head is not complete;
    3. a `next` that waits, or that is called later inside a `for ever` body, fails with `Interrupted`, so the accept loop ends at its next iteration start and its `on stop` runs;
    4. started tasks are not interrupted (ADR 0116 rule 14 item 1): they drain, and every reply they write carries `Connection: close`;
    5. the `tasks` statement ends once every started task has ended, then the code after it runs, `on_stop` for `[serve]`, and the run ends with exit code 0;
    6. at the hard stop (ADR 0116 rule 16), every task is cancelled, the host sends the auto-answers of their reply rights (rule 4.5) without waiting on any peer, and the run ends with U6007.

    ```text
    SIGTERM → listeners closed → next fails Interrupted → on stop → tasks drain → exit 0
    ```

### G. Runtimes and tests

19. **Native code.**
    - **Threads.** The workers of ADR 0117 rule 2 serve the connections: each worker has its own `kqueue` or `epoll`, owns the connections it accepted, and does their TLS, reading, parsing and writing between the coroutines it runs. The system calls live where ADR 0117's reactor puts its own.
    - **One listening socket per grant.** Each grant binds one socket, without `SO_REUSEPORT`, so no other process can bind the same address and port and take a share of the connections. On Linux every worker waits on it with `EPOLLEXCLUSIVE`, which limits the wake-ups of blocked workers to one per connection; a worker that polls without blocking may also see the socket ready, and its `accept` then fails with `EAGAIN`, which it ignores. On macOS the socket is registered with `EV_DISPATCH` in a `kqueue` for accepting, nested in each worker's own: every worker may wake, one receives the event, accepts at most 16 connections, and enables it again, so a burst spreads over the workers. The worker that accepts a connection owns it.
    - **Handshakes.** A worker runs a TLS handshake only while fewer than half of the workers, rounded up, are running one (a shared counter); otherwise the connection waits in that worker's queue of handshakes, within `read`, so request handling always keeps the other workers.
    - **Placement.** A started task whose captures hold a reply right runs on the worker that owns the reply right's connection, instead of the next worker in turn (ADR 0117 rule 2, Appendix A). Each task stays pinned, and there is no work stealing (ADR 0117, Consequences): a worker with slow handlers delays the other requests of its own connections.
    - **What is shared, and what order is kept.** Per worker: its connections, its readiness queue, its queue of requests read and waiting, and its handshakes. Shared by the workers of a grant, as atomic counters on their own cache lines: the count of open exchanges against `/once`, the GCRA timestamp of the rate, the count of waiting requests against `waiting`, the bytes reserved against `buffers` and the peer table. Each request costs a few atomic operations on them, O(1), and no lock. First come, first served holds per worker: each worker delivers its own waiting requests in the order it read them. Across workers the order is not kept, so a busy worker's waiting requests may lose `/once` slots and tokens to other workers' newer ones. Unlike the waiters of a limit in ADR 0117 rule 8, which none starves, such a request is bounded only by `answer`, after which it gets `503`.
    - **The `[serve]` path.** The driver of rule 16 runs as one accept loop per worker, each delivering the requests of its own connections to tasks on its own worker, so a request is read, handled and answered on one core, with no hand-off of the request between threads. It is the same program under every schedule, since requests arrive in an order that only timing decides. The paths of its started tasks (ADR 0117 rule 7) take their number `k` from one shared counter, one atomic increment per request. Under `--virtual-time` the native run uses one worker (ADR 0117 rule 12), and the driver is then one loop, as written in rule 16.
    - **The language form** delivers each request to the task that calls `next`, which may run on another worker, and its `start` places the request's task back on the connection's worker: two hand-offs between threads per request.
    - **Values.** The worker builds the request's bytes; `next` turns them into Una values in the task's heap, one copy, with the UTF-8 check done while parsing.
20. **The interpreter** runs the same host code, from `crates/una-http`, on serve threads of its own, `--serve-threads <n>` from 1 to 256, by default the number of cores. Only plain owned data crosses to the interpreter's thread, as for its other waiting operations (ADR 0117 rule 11). Its started tasks are the coroutines of ADR 0117 rule 11.
21. **Tests and equivalence.**
    - **`std.testing`** gains `test_request(method: Method, target: Text, body: Text) -> Request fails TestRequestError`, which reads `target` by rule 11, with no headers but `host: test`, `peer` `"192.0.2.1"` and `listener` `"test"`, and fails with `Refused` where the host would answer `400`; and `test_exchange(request: Request) -> Exchange`, which gives a fresh reply right. `std.testing` declares `pub kind TestRequestError = Refused`.
    - **Handlers.** A test may install a handler for `HttpServer`. Handlers have no mutable state (`spec/05 §5` rule 3), so the usual test of a server calls `handle`, or the function a started task runs, directly.
    - **Scripted connections.** `una run` and executables take `--serve-script <file>`, which requires `--virtual-time` and replaces every listener with the connections the file describes, one JSON object per line, with `at` in virtual milliseconds:
      - `{"at": <ms>, "connection": <k>, "grant": "<address>", "peer": "<ip>", "reads": "eager" | "paced"}` opens connection `k` on a grant from a peer address; a `paced` connection receives reply bytes only as its `read` events allow;
      - `{"at": <ms>, "connection": <k>, "bytes": "<base64>"}` sends bytes;
      - `{"at": <ms>, "connection": <k>, "read": <n>}` lets a `paced` connection receive `n` more bytes, which models a peer that reads slowly;
      - `{"at": <ms>, "connection": <k>, "close": true}` closes the peer's side.

      Events with equal `at` take effect in the order of their lines. `Date` is written from the virtual clock. `--serve-out <file>` writes the bytes each connection received, with the virtual time each byte left, in the same form. The real parser, limits, shares, timers and writer run on these bytes, so `spec/16 §5` compares the two engines on them, byte for byte.
    - **What only live tests cover.** A grant's `https` scheme is served as plain bytes under a script, so the TLS rules (the handshake, its rate and share, SNI against `Host`, the protocol versions, early data, ALPN) are covered by the live tests of the Conformance section, not by scripts.
22. **The report.**
    - The host capabilities list `HttpServer`, with each grant, its limits, its scheme, and whether it checks `Host`.
    - The `tcb` section lists the server code of `una-http` and, for `https` grants, the TLS server of `rustls`.
    - The `out_of_model` list gains, at its end, "what a server's peers send in reaction to its timing, and which requests its full queues refuse" and "what a peer receives when the host's deadlines cut an exchange", shown for packages with a `serve:` grant or a `[serve]` table.
    - Warnings, in the report only: a plain `serve:http://` grant on an address that is not loopback; a grant with `hosts=*`, `peer=none` or `peer_buffers=none`; a grant with `tls12`.
    - These are additions within `una.report/2`.

## Verification

- **`Status`.** Every `Reply(status: s, ...)` carries the refinement obligation `200 <= s and s <= 599` (`spec/12 §3`, refinement).
- **No `requires` on `reply`.** A precondition over the reply's body would be unknown (U5002) at every `reply` of a value that a function returned without an `ensures`, so the rule that a `204` or `304` has no body is checked by the host, as `BadReply` (rule 13), a failure of the program's own data.
- **No fact about the network.** No operation has an `ensures`, so nothing is assumed about requests, and the host is trusted only to deliver what rules 2, 10 and 11 say, which its tests check.
- **No new obligation kind.** Reply rights are host state, outside the verifier's encoding, as the world is (`spec/19 §4` rule 4).

## Information flow

IF1. **Requests are input.** `next` returns an `internal Exchange`, as `Http` returns an `internal Response` (`spec/24 §5`) and `Files.read` an `internal Text` (`spec/14 §3`). Whether `next` succeeds is `public`, as whether `Env.get` finds a variable is (`spec/14 §3`): its failures are decided by no argument (`spec/07 §7` rule 4), but by host stops, which are input under a fixed stop schedule (ADR 0116, Information flow), by the program's own `close`, which accepts `public` only and so runs only under a `public` program counter, and by deadlines, which ADR 0116 IF8 and ADR 0117 IF2 already label. So an accept loop stays at a `public` program counter, and it can `stop when` and print.

IF2. **Replies accept `internal`.** Every argument of `reply` is a sink with `accepts internal` (`spec/07 §7` rule 4): `pc ⊔ ℓ⁺(arg) ⊑ internal`, else U4003. A `secret` value reaches a reply only through a policy (`spec/07 §5`). A reply goes to the peer that sent the request, which is the destination the program chose, as `Http`'s origin is (ADR 0074 H3).

```una
balance_reply(request: internal Request) -> internal Reply
  uses @accounts: Accounts[balance]          -- returns `secret Int`
=
  let cents = @accounts.balance(of: request.body)
  Reply(status: 200, headers: [], body: "{cents}")   -- U4001: a `secret` value in the `internal` field `body`
```

IF3. **Failures of `reply`.** `NotHeld` and `BadReply` are decided by `to` and `reply`, which accept `internal`, and by the task's own reply rights, which rule 4 makes a function of the program's order. So the failure is labeled `internal`, as `Http`'s are (`spec/24 §5` rule 3): `attempt` gives an `internal` `Result`, and `if error pass` and `if error fail` are U4002.

IF4. **No host deadline becomes a value.** The host's `answer`, `read`, `idle` and `write` deadlines, a peer that leaves, full queues and the limits of rule 7 change only what the network carries: none of them is a result or a failure of an operation (rule 14). The values the program sees that timing can change are `next`'s `TimedOut` under the program's own deadline, which ADR 0116 IF8 and ADR 0117 IF2 label, and the requests themselves (IF5).

IF5. **The request stream** is input from outside (Question 4). Its dependence on the server's own timing, through peers that measure its replies, and through the requests that the host itself refuses when the program's handlers are slow (full `waiting`, `buffers` or `connections`), is outside the model, as timing and external systems are (`spec/07 §1`), and the report says so (rule 22). This is a deliberate exception to ADR 0117 IF4, which labels the refusals of a limited `Http` grant with the timing of every site that uses it; it is not the position of `Http`. What it leaves unbounded is what `internal` code may learn about the timing of the program's own handlers from which requests arrive. `Public` values stay unaffected: the accept loop branches only on whether `next` returned a request, and how many iterations ran before a stop is an input of the stop schedule (ADR 0116, Information flow).

IF6. **Auto-answers.** A task that ends holding a reply right answers it through the host, so its occurrence and status depend on how the task ended. They are checked as a `reply` at the `start` that moves the reply right: a `start e` whose captures can hold a `ReplyRight`, by their types, or include a function value, which may have captured one, is checked under `pc ⊔ τ(e)` when it has `within`, and under `pc` otherwise, against `internal` (U4003 at the `start`). A reply inside `e` already meets the same bound under ADR 0117's timing floor, so this rejects only a `start` whose expression never replies.

IF7. **Labels in a server, in practice.** A request task branches on `internal` data, so its program counter is `internal`: it can call `Http`, `Files.write`, `limiter` and `reply`, which accept `internal`, and cannot print `public` lines. Its tasks make `τ_conc` at least `internal` beside them (ADR 0116 IF6), so deadlines beside them fail with `internal` labels, which `internal` handling clauses take. A task that reads `secret` data, from a capability whose results are `secret`, cannot run under `within`, as ADR 0117's acceptance decision 2 states; `[serve]` therefore has no `within` by default (rule 16).

**Where this sits in the model.** T6 holds for each task's values and events, for the result of each construct, and for a fixed stop schedule, as ADRs 0116 and 0117 state, with the request stream as input. What a peer measures, and what the host's deadlines tell it, is timing, outside the model. Labels do not separate one peer from another: an `internal` value from one peer's request may go into another peer's reply, so a server that must keep its users' data apart does so in its own code, and a later record on labels per principal could check it.

## Security

- **Authority.**
  - A server listens only on the addresses its grants name, bound before `main` starts; host names are refused, so nothing resolves a grant.
  - No connection, socket or key is a value. A reply right names an exchange, but replying needs `@server` with `reply`, which flows downward by name as every capability does (`spec/05 §4`).
  - `close` accepts `public` only.
  - No operation of `HttpServer` runs in a `together`, `together for` or `first of` task (rule 5).
- **Request smuggling and parser confusion.** Every ambiguous form is refused and closes the connection (rule 10); the framing functions are strict and shared with the client, which re-frames every request it sends, so a proxy written in Una never passes a peer's framing on; the host reads one request at a time per connection; it strips the connection's headers before the program sees the request, and refuses a `Connection` header that names others (rule 2).
- **Header injection and response splitting.** A reply's header names and values are checked before anything is sent, and the host writes the framing headers itself (rule 13).
- **Path confusion.** Segments are decoded one by one, and `.`, `..`, encoded `/`, `\` and control characters are refused (rule 11), so `segments` can be joined with `/` again without changing the path. Forms that other servers read as special pass as ordinary segments, such as `..;` and the double-encoded `%252e%252e`: correct for Una, but a proxy written in Una that forwards a path to a server that normalizes differently should refuse them itself.
- **DNS rebinding.** Without `hosts=`, every grant accepts only `localhost` and IP literals in `Host`, and `Host` syntax is checked on every grant (rule 8).
- **The listening socket.** One per grant, without `SO_REUSEPORT`, so another process cannot share the port (rule 19). On macOS and the BSDs, another process can still bind a more specific address on the same port, such as `127.0.0.1:p` beside `0.0.0.0:p`, with `SO_REUSEADDR`; that is among the other processes on the same ports, out of model.
- **Header values in traces.** Credentials of peers are never written to a trace or a recording, and bodies only by length and a keyed hash (rule 1).
- **Denial of service.** Every resource is bounded by default: connections in all and per peer, IPv6 peers counted per /64; TLS handshakes per second, in all and per peer, on at most half of the workers; bytes per request line, header section and body; header lines; requests per connection; bytes of bodies in all and per peer, with a reserve per connection for its head, refused with `503` when they do not fit; bytes of unwritten replies in all and per peer, outside the run's memory budget, where the slowest connections are closed under pressure, so slow uploaders and slow readers cannot stall other peers or end the run (rule 13); requests waiting, in flight and delivered (`delivered`); time to idle, read, write and answer (rule 7); the program's own tasks and memory (ADR 0117); and compression is refused (rule 10). Time totals bound how long one connection is held; the per-peer limits bound how many one attacker holds (rule 12). The worst-case memory of a grant is stated in rule 13. A request that recurses too deep ends its task alone and gets `500` (ADR 0117 rule 7, rule 4.5). No malformed request or reply aborts the server: the host refuses the one, and the other fails with `BadReply`, not U6004.
- **TLS.** Keys stay in the host, are read from the descriptor whose owner, group and mode were checked, and appear nowhere else. TLS 1.3 only unless `tls12`, which requires the extended master secret. No renegotiation, no early data, no stateless tickets, no key log. Handshakes are limited per second, per grant and per peer, and run on at most half of the workers (rules 9 and 19).
- **Effects in doubt.** A reply may not reach its peer: the host's deadline may have passed, or the peer may have left (rule 14). The program sees `unit` in both cases. A server that must know whether its peer received something uses an idempotent protocol, as with any network failure (ADR 0116 rule 12).
- **Out of model, stated.** What peers learn from timing (IF5); separation between peers (Information flow); the effect of other processes on the same ports; a peer's own lies about `peer` behind a proxy, which the program must not trust from headers; an attacker with a whole IPv6 /48 or many addresses, whom only the grant's totals bound (rule 12); credentials in paths or in headers the trace does not know (rule 1).
- **Trust.** The server code in `una-http` (parser, writer, limits, TLS setup) joins the trusted base, under a security review before it merges (ADR 0121 rule 2 (b), draft). The `serve:` grant is the package's opt-in (ADR 0121 rule 2 (a), draft). No `unsafe` code beyond what ADR 0117's reactor already places.

## Costs

| Operation | Time | Space |
| --- | --- | --- |
| Accept a connection | O(1), plus the TLS handshake for `https`: one key exchange and one signature | one descriptor; a head reserve counted at accept but allocated only as bytes arrive, and released when idle |
| Read and parse a request | O(b) for its b bytes, one pass, each byte examined a bounded number of times, the UTF-8 check fused into it | O(b), bounded by rule 7 |
| Decode segments and query | O(target length) | O(target length) |
| Queue a request, deliver it by `next` | O(1) each | one queue entry |
| Build the `Exchange` in the task's heap | O(b): one copy | O(b) |
| Hold, move and release a reply right | O(1) each: an index and a generation in a table | one table slot per open exchange |
| Move reply rights at `start` | native: none beyond the copy of the captures (ADR 0113); interpreter: O(size of the captured values whose type can hold a `ReplyRight` or a function). If ADRs 0107 and 0108 (Proposed) share values between task heaps and remove that copy, the walk costs the same O(size) in native code too | none |
| `reply` | O(r) for the r bytes of the reply: checks and serialization into one buffer | O(r) until written |
| Limits and shares | O(1): a few atomic operations on counters and one GCRA timestamp per grant, and one entry of the peer table | O(1) per grant, O(connections) for the peer table |
| Closing for reply pressure | O(log c) per reply charged, for a heap of the c connections with pending replies ordered by bytes left, kept per peer and per grant | O(c) |
| Timers | O(1) to arm and cancel, in the timing wheel of ADR 0117: at most one per connection and one per open exchange | one each |
| Auto-answers at a task's end | O(reply rights it holds) | none |
| Shutdown | O(connections + queued requests) | none |

- **Per request on the `[serve]` path,** keep-alive, plain HTTP, small request: one `read` and one `write` system call, the readiness wait amortized over the ready connections, the parse, a few atomic operations on the grant's shared counters, one coroutine started from the stack pool, the handler and the serialization, with no hand-off of the request between threads (rule 19). The shared counters are the cost that a server with limits per worker would not pay; a measurement decides whether they need per-worker batches.
- **The language form** adds two hand-offs between threads: to the task that calls `next`, and back to the connection's worker through the placement of rule 19, two queue operations, O(1) each, and a wake-up of the other worker when it sleeps.
- **Header lookup** in the program is a scan of `headers`, O(h), which rule 7 bounds by 100 lines by default.
- **The goal and how it is measured.** The project's bar for native code, parity with Rust (a geometric mean of at most 1.1x Rust's time, no program above 1.5x, and memory at most 1.2x), was set for compute programs (ADR 0106, Context). This record sets the same bar for servers, on CPU time per request at saturation and on memory, against a like-for-like baseline, and reports latency without a bar.
  - **The baseline.** A Rust server on `hyper` over HTTP/1.1, on the multi-threaded `tokio` runtime, with the same work and the same limits: `tower`'s `ConcurrencyLimit` and `RateLimit` for `/once` and the rate, `http-body-util`'s `Limited` for `body`, a timeout on reading the head, a cap on connections, and a handler that checks UTF-8 and decodes the path and the query as rule 11 does and builds the same lists. A bare `hyper` server is reported beside it, to show what the checks cost.
  - **The programs.** A health endpoint and a JSON endpoint, with keep-alive and without, plain and TLS, on the same machine.
  - **The records.** CPU time per request at saturation; the 50th and 99th percentile latency at fixed loads, with uniform handler costs and with skewed ones (one request in a hundred a hundred times slower); and memory per idle connection. Any program above the bar is stated with its reason.
  - **The expectation, stated as such.** Within the bar on the `[serve]` path at uniform costs, whose structure is that of a thread-per-core Rust server, less the shared counters. Above `tokio` at the 99th percentile under skewed costs, since pinned tasks without work stealing wait behind a slow handler on their worker while `tokio` moves them. The language form may sit above 1.1x on the smallest requests, where its two hand-offs are a large share of the work.
- **Scaling tests,** native code: 1,000, 10,000 and 50,000 idle keep-alive connections (memory linear, no time growth per request); 1,000 to 50,000 entries in the peer table (O(1) per accept); 1,000 and 10,000 requests in flight, each parked on an `Http` call; headers of 1 to 64 KiB and 1 to 1000 lines (parse time linear); bodies in chunks of 1 byte up to the limit (linear); a request split at every byte boundary (same result). A quadratic step fails these tests.

## Diagnostics

One new code, which joins `spec/13 §4` with its implementation. U2034, U2035, U3027 to U3029 and U6007 belong to ADRs 0116 and 0117; U2036 to U2039, U3030 to U3032, U4009, U6008, U6009 and U7006 to U7014 to drafts in review. U3033 is free on `main`, in the open pull requests and in the drafts.

| Code | Domain | Meaning |
| --- | --- | --- |
| U3033 | authority | An operation of `HttpServer` in the effect set of a `together` branch, a `together for` body or a `first of` branch |

Its message: "`@server.reply` cannot run in a branch of `together`; answer after the block". Its repair, with no edit: move the operation after the construct.

Existing codes cover the rest:
- **U1001:** an import of these names in an earlier edition.
- **U3014:** an example that uses `HttpServer` without a `given` handler.
- **U3012:** a `uses` of `HttpServer` with no `serve:` grant, and the `uses` of `handle` and `on_stop`.
- **U3027, U3029:** `diverges` on `handle` or `on_stop`, and `fails` on `on_stop`.
- **U4001, U4002, U4003:** IF2, IF3, IF6.
- **U5001, U5002:** the refinement of `Status`.
- **U7002:** an invalid `serve:` grant in `una.toml`, and an invalid `[serve]` table.

Usage errors, exit code 2: an invalid `--grant serve:...`; an address that cannot be bound; unreadable, malformed or mismatched TLS files, a key file that is not a regular file, is owned by another user than the run's or root, or is open to group or others, and an `https` grant on Windows; a `[serve]` module without `handle`, or a run with `[serve]` and no `serve:` grant; `--serve-threads` out of range; `--serve-script` without `--virtual-time`, or a script that is not valid; a descriptor limit below `connections` plus 64 that the run cannot raise.

## Consequences

- **What can be written.** HTTP/1.1 services, plain or with TLS, as a function `handle(request) -> Reply` that the host runs, or as a loop in the language; with every resource bounded by default, strict parsing, and a graceful stop on SIGTERM.
- **What labels say about servers.** Requests are `internal` and replies accept `internal`, so a server cannot send `secret` data without a policy, and cannot print request data to `Console`. A server that logs needs an `internal` sink; `Files.write` accepts `internal` but rewrites a whole file, so an append-only log capability is a natural next record.
- **Configuration that secure defaults ask for** (Question 7): servers reached by name list their names in `hosts=`; servers behind a proxy set `peer=none`, `peer_buffers=none` and `peer_replies=none`.
- **Future work:** stricter framing for the client, in a later edition, if wanted (rule 10).
- **What waits:** raw TCP and binary bodies (Question 1); HTTP/2 and WebSockets; streaming bodies; client certificates and certificate reload (Question 3); limits per peer on the rate of requests, and keyed limits in code; an access log written by the host; labels per principal.
- **Tests** of servers call `handle`; runs are compared between engines with scripted connections under virtual time.
- **Earlier editions** do not change.

## Alternatives considered

- **A `Listener` of raw connections now** (Question 1). Not chosen: without bytes and streams it would push HTTP parsing into Una code, and every server would carry its own smuggling risks.
- **Connections as values,** as the sketches of ADRs 0116 and 0117 suggest. Not chosen: a connection carries many requests under keep-alive, it would be authority stored in a value, and its lifetime would have to be tracked by the program. Reply rights name one exchange and carry no authority.
- **A static exactly-once rule for reply rights** (a linear type). Not chosen: a new kind of type for one capability; the dynamic rule of rule 4 is deterministic and ends every exchange.
- **`reply` failing when the host's deadline passed, or the peer left.** Not chosen: the failure would turn the program's timing into a value (IF4), and need a timing label on every reply.
- **The request stream labeled by the server's timing** (Question 4 (b)). Not chosen: servers that touch `secret` data could not exist.
- **Lenient parsing,** as RFC 9112 permits in places. Not chosen: leniency is where two implementations disagree (Question 6).
- **TLS keys through `Keys`.** Not chosen: a different format and use, and an operation the program does not need (Question 3).
- **`SO_REUSEPORT` with a socket per worker.** Not chosen: another process of the same user could bind the same port and take a share of the connections (rule 19).
- **Accepting every `Host` without `hosts=`,** except on loopback. Not chosen: a grant on `0.0.0.0` or a private address, the usual setup in containers, stays open to DNS rebinding (rule 8).
- **One shared budget for all request bytes.** Not chosen: a few slow peers would fill it and stop the reading of every connection (rule 13).
- **Host names in grants.** Not chosen: a grant would depend on name resolution at start-up, and a server's listening address is the operator's to state.
- **The kinds in the prelude,** as `spec/24`'s are. Not chosen: `Request` and `Reply` are common names that edition `0.4` would reserve; an import costs one line.
- **A thread per connection,** or a pool of helpers for connections as ADR 0117's first step has for `Http`. Not chosen: a slow peer would hold a thread, and slowloris would exhaust the pool; the server needs the reactor from its first day, as ADR 0117's plan already orders.
- **`[drive]` with a request input.** Not chosen: a step answers one input at a time on one state, and a server answers many at once.

## Implementation plan

The server lands after the coroutines, `within`, `tasks` and `start`, and the reactor of ADR 0117 (its implementation plan, steps 1 to 6).

1. **Strict framing for the server,** in `crates/una-http` beside `parse.rs`, whose client functions do not change: a chunk size of 1 to 16 hex digits that fits in 64 bits with no extension; one `Content-Length`; no repair of a value that is not UTF-8; trailers within the limits of the head.
2. **The parser and the writer of the server,** with no I/O: rules 10, 11 and 13, using the framing functions of step 1. Fuzz and property tests come first: every request split at every byte boundary; every refusal of rules 10 and 11; a corpus of request-smuggling, hop-by-hop and path-confusion cases from RFC 9112 §11 and published research on desynchronization; differential tests of the client's and the server's framing on one corpus, compared on the inputs both accept.
3. **Grants:** the `serve:` spec in `crates/una-core/src/http.rs`, `una.toml`, `--grant`, overlap checks, binding before `main`, the TLS files and their checks on the open descriptor, the pinned TLS configuration.
4. **The serve threads:** one listening socket per grant with exclusive wake-ups, per-worker readiness, the shared counters, the peer table, the reserves and shares of requests, replies and delivered requests, the closing of the slowest connections under reply pressure, the handshake rate and share, timers, `Host` checks, the host's own answers, the closing after a refusal, shutdown.
5. **The capability in both engines:** the declarations, imported only in edition `0.4`; `next`, `reply`, `close`; reply rights, their moves at `start`, the state machine of rule 4.7, the placement of rule 19, auto-answers and their trace events; the reduced trace form of rule 1; U3033; IF6.
6. **`[serve]`,** its checks, and its per-worker driver.
7. **Tests:** `test_request`, `test_exchange`, `--serve-script` with its peers, pacing and ordering, `--serve-out`, and the equivalence of rule 21.
8. **Examples and documents:** the three servers of this record under `examples/`, and a guide page.
9. **Performance:** the server programs, the like-for-like baseline and bare `hyper`, and the scaling tests (Costs).
10. **Security:** a review of the server code before it merges; generators for the hunt (`docs/plan/security-hunt-generators.md`) for malformed requests and for replies with injected headers; a run of the two-run oracle over handlers that read labeled data.
11. **The formal model** (`formal/UnaFormal/F1`): `next` as an input stream and `reply` as an effect; T6 under a fixed stop schedule with the stream as input; a lemma that which task holds a reply right is a function of each task's own order. The parser is outside the formal model, and its fuzzing and differential tests stand for it.

## Conformance

- **Names and editions:** the imports in edition `0.4`, and U1001 in `0.3`; a user kind `Request` beside an import of `std.host (Reply)`; `serve:` grants refused in `0.3`.
- **Checks:**
  - U3033 for `reply` in a `together` branch, for `next` in a `first of` branch, and for a call in a `together for` body of a function whose effects include `reply`;
  - U3012 for `uses @server: HttpServer` with no `serve:` grant;
  - U5001 for `Reply(status: 700, ...)`;
  - U7002 for each invalid grant item and `[serve]` table; usage errors for each listed case.
- **Information flow:**
  - a `secret` body in a reply (U4001 or U4003);
  - `close` under an `internal` program counter (U4003);
  - `if error pass` on a `reply` failure (U4002);
  - an accept loop that prints a `public` count of requests (accepted);
  - a `start ... within` whose expression computes on `secret` data and never replies (U4003 at the `start`, IF6);
  - a request task that reads `secret` data and replies after a policy, without `within` (accepted).
- **Runs, in both engines, under `--virtual-time` with `--serve-script`, compared byte for byte:**
  - the three example servers;
  - keep-alive, `Connection: close`, `HTTP/1.0`, pipelined requests answered in order;
  - each refusal of rules 8, 10 and 11, with its status and the closed connection, `Connection: close, x-forwarded-for` and `Host: a/b@c` included, and a `Host` that is a DNS name on a grant without `hosts=` (`421`);
  - 64 `paced` connections that never read their 1 MiB replies, 64 that never read 3 MiB replies, and 64 that trickle 1 MiB bodies, while a fresh client's request is answered within its own time, with no U6003 and the slowest connections closed;
  - a program that holds 80% of `--max-memory` in its own state still delivers and answers requests;
  - a body larger than what remains of `buffers` or `peer_buffers`, answered `503` at once;
  - a peer at `peer` connections, and two IPv6 peers in one /64 counted as one;
  - `idle`, `read`, `write` and `answer` passing, and a reply discarded after `answer`;
  - `waiting` full, `/once` and a rate limit delaying delivery;
  - a forgotten reply (`500`), a task cut by `within` (`503`), a second reply (`NotHeld`), a reply with `content-length` or a `204` with a body (`BadReply`);
  - a host stop with requests queued, in flight and idle, ending with exit code 0, and the hard stop ending with U6007 and `503`s.
- **Live sockets, both engines:** a request over loopback, plain and with TLS against a test authority made per test process, as the client's tests make theirs (ADR 0074, amendment of 2026-10-07 "no private keys in the repository"); SIGTERM draining one slow request.
- **TLS:** a key file with mode `0644` refused, one owned by another user refused, one reached through a symbolic link with mode `0400` accepted; a mismatched key refused; TLS 1.2 refused without `tls12` and accepted with it; early data refused; ALPN `h2` alone refused; a flood of handshakes above `handshakes` closed at accept while requests on open connections are answered.
- **Traces:** a request with `authorization` and `cookie` and a reply with `set-cookie`, whose trace and recording show `<credential>`, and bodies shown by length and keyed hash.
- **Scale and performance:** the tests of Costs.

## Edition `0.4`

As ADR 0121 rule 10 (draft) asks:
- **Words:** none.
- **Clashes:** none in the language. The names are imported only (rule 0), and `serve:` grants and `[serve]` were errors in earlier editions. The client of `spec/24` does not change.
- **Migration:** none.
- **Unsettled points:** none (rule 7).
- **Trust:** under rule 2 (a), the `serve:` grant is the opt-in; under rule 2 (b), the server code of `una-http` and the TLS server of `rustls`, with a security review.
- **Security:** Information flow, and the `out_of_model` entries of rule 22, shown only for packages that serve.

## Examples

### A health endpoint (`[serve]`)

```toml
[package]
name = "health"
version = "0.1.0"
edition = "0.4"

[serve]
module = "health"

[run]
grants = ["serve:http://127.0.0.1:8080"]
```

```una
module health

import std.host (Request, Reply)
import std.testing (test_request)

--- Answers `GET /health` with 200, and every other request with 404.
pub handle(request: internal Request) -> internal Reply
=
  if request.method is Get and request.segments == ["health"]
    Reply(status: 200, headers: [], body: "ok")
  else
    Reply(status: 404, headers: [], body: "")

example "the health endpoint answers ok"
  do handle(test_request(method: Get, target: "/health", body: ""))
  expect result.status == 200
```

### A JSON API over a database-like capability (`[serve]`)

The orders live in a document database with an HTTP interface. The program declares the capability it needs, and a handler implements it over `Http`; tests install an in-memory handler instead.

```toml
[package]
name = "orders"
version = "0.1.0"
edition = "0.4"

[serve]
module = "orders.api"

[run]
grants = [
  "serve:https://0.0.0.0:8443;certificate=tls/chain.pem;key=tls/key.pem;256/once;6000/minute;body=64;hosts=orders.example",
  "http:http://127.0.0.1:5984:get,put;200/second;32/once",
  "keys:keys.toml",
]
```

```una
module orders.store

import std.host (Header)

pub kind Order
  id: internal Int
  item: internal Text
  quantity: internal Int

--- What a lookup found. An `Http` failure is `internal`, so the store reports it as a value.
pub kind Lookup = Found(order: internal Order) | Missing | Unavailable

pub kind Saved = Stored | Refused

pub capability Orders
  op find(id: Int) -> internal Lookup
    accepts internal
  op add(order: Order) -> internal Saved
    accepts internal

--- The orders, in a document database at `base`, one document per order.
pub handler document_store(base: Text) for Orders
  uses @http: Http[get, put]
  op find(id) =
    let answer = attempt @http.get(url: "{base}/orders/{id}", headers: [], key_name: Some("store"))
    when answer
      is Ok(response) -> lookup_of(id, response)
      is Err(failure) -> Unavailable
  op add(order) =
    let answer = attempt @http.put(url: "{base}/orders/{order.id}", headers: [json_type()], body: order_json(order), key_name: Some("store"))
    when answer
      is Ok(response) -> if response.status == 201 then Stored else Refused
      is Err(failure) -> Refused

pub json_type() -> Header = Header(name: "content-type", value: "application/json")

lookup_of(id: internal Int, response: internal Response) -> internal Lookup
=
  if response.status == 404
    Missing
  else
    when order_from(id, body: response.body)
      is Some(order) -> Found(order)
      is None -> Unavailable

--- Reads `{"item": "...", "quantity": n}`.
pub order_from(id: internal Int, body: internal Text) -> internal Option[Order]
=
  let parsed = attempt body.parse_json()
  when parsed
    is Err(failure) -> None
    is Ok(json) ->
      when json
        is Object(fields) ->
          let item = fields.get(key: "item")
            if none use Null
          let quantity = fields.get(key: "quantity")
            if none use Null
          when item
            is Str(name) ->
              when quantity
                is Number(count) -> if count > 0 then Some(Order(id: id, item: name, quantity: count)) else None
                else -> None
            else -> None
        else -> None

--- Writes the order by interpolation: the fields of `Json` are `public`, so a `Json` value cannot hold `internal` data.
pub order_json(order: internal Order) -> internal Text
=
  "\{\"id\": {order.id}, \"item\": {order.item.quote_json()}, \"quantity\": {order.quantity}\}"
```

```una
module orders.api

import std.host (Request, Reply, Header)
import orders.store (Orders, Lookup, Saved, document_store, order_from, order_json, json_type)

--- `GET /orders/<id>` reads an order; `POST /orders/<id>` stores one.
pub handle(request: internal Request) -> internal Reply
  uses @http: Http[get, put]
=
  route(request) with @orders = document_store(base: "http://127.0.0.1:5984/shop")

route(request: internal Request) -> internal Reply
  uses @orders: Orders[find, add]
=
  let path = request.segments
  let id = path.at(1)
    if none use ""
  if path.len != 2 or path.at(0) != Some("orders")
    text(404, "no such resource")
  else
    when id.to_int()
      is None -> text(400, "the id is not a number")
      is Some(number) ->
        when request.method
          is Get -> show(number)
          is Post -> create(number, body: request.body)
          else -> Reply(status: 405, headers: [Header(name: "allow", value: "GET, POST")], body: "")

show(id: internal Int) -> internal Reply
  uses @orders: Orders[find]
=
  when @orders.find(id: id)
    is Found(order) -> Reply(status: 200, headers: [json_type()], body: order_json(order))
    is Missing -> text(404, "no such order")
    is Unavailable -> text(503, "the store is unavailable")

create(id: internal Int, body: internal Text) -> internal Reply
  uses @orders: Orders[add]
=
  when order_from(id, body: body)
    is None -> text(422, "expected an item and a positive quantity")
    is Some(order) ->
      when @orders.add(order: order)
        is Stored -> Reply(status: 201, headers: [Header(name: "location", value: "/orders/{id}")], body: "")
        is Refused -> text(503, "the store refused the order")

text(status: Status, message: Text) -> Reply = Reply(status: status, headers: [], body: message)

example "an unknown order is 404"
  given @orders = no_orders()
  do route(test_request(method: Get, target: "/orders/7", body: ""))
  expect result.status == 404

handler no_orders() for Orders
  op find(id) = Missing
  op add(order) = Refused
```

### A rate-limited proxy (the language form)

Inbound requests are paced by the grant (600 a minute delivered, 256 open) and by a `limiter` in code (20 a second for the upstream); upstream calls are paced again by the `Http` grant (ADR 0117 rule 8), and each request is cut at 5 seconds.

```toml
[package]
name = "proxy"
version = "0.1.0"
edition = "0.4"

[run]
entry = "proxy.main"
grants = [
  "serve:https://0.0.0.0:8443;certificate=tls/chain.pem;key=tls/key.pem;256/once;600/minute;body=0;hosts=proxy.example",
  "http:https://upstream.example:get;100/second;64/once",
  "keys:keys.toml",
]
```

```una
module proxy.main

import std.host (HttpServer, Exchange, Request, Reply, Header)
import std.limit (Limit, limiter)
import std.time (second)

pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @http: Http[get]
  diverges
=
  serve_all() with @gate = limiter(at_most: 20, per: second, waiting: 100)

serve_all() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @http: Http[get]
  uses @gate: Limit[acquire]
  diverges
=
  tasks at most 512 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start forward(exchange) within 5 * second
            if error use unit                     -- cut: the host answers 503
        is Err(failure) -> unit
    on stop
      @server.close()

forward(exchange: internal Exchange) -> Unit
  uses @server: HttpServer[reply]
  uses @http: Http[get]
  uses @gate: Limit[acquire]
=
  @server.reply(to: exchange.right, reply: answer_for(exchange.request))
    if error use unit

answer_for(request: internal Request) -> internal Reply
  uses @http: Http[get]
  uses @gate: Limit[acquire]
=
  if not (request.method is Get)
    Reply(status: 405, headers: [Header(name: "allow", value: "GET")], body: "")
  else
    let paced = attempt @gate.acquire()
    when paced
      is Err(failure) -> Reply(status: 429, headers: [Header(name: "retry-after", value: "1")], body: "")
      is Ok(done) ->
        let upstream = attempt @http.get(url: "https://upstream.example{request.target}", headers: outbound(request.headers), key_name: Some("upstream"))
        when upstream
          is Ok(response) -> relay(response)
          is Err(failure) -> Reply(status: 502, headers: [], body: "")

--- Only the headers the upstream needs, by an allowlist: a peer's `forwarded`, `x-forwarded-*`, credentials and `expect` never reach it.
outbound(headers: internal List[Header]) -> internal List[Header]
=
  headers.filter(keep: fn(h) -> ["accept", "accept-language", "if-none-match", "if-modified-since", "user-agent"].contains(h.name))

relay(response: internal Response) -> internal Reply
=
  if response.status >= 200 and response.status <= 599
    Reply(status: response.status, headers: inbound(response.headers), body: if response.status == 204 or response.status == 304 then "" else response.body)
  else
    Reply(status: 502, headers: [], body: "")

--- Drops the headers the host writes itself, whose names the client keeps as received.
inbound(headers: internal List[Header]) -> internal List[Header]
=
  headers.filter(keep: fn(h) -> not ["connection", "content-length", "date", "keep-alive", "proxy-connection", "te", "trailer", "transfer-encoding", "upgrade", "set-cookie"].contains(h.name.to_lower()))
```

The labels hold without a policy: requests and upstream responses are `internal`, and `reply`, `Http` and `acquire` accept `internal`. The `within` on `start` checks `forward` under the timing floor of ADR 0117 IF2 with `τ` at `internal`, which those sinks accept, and IF6 holds since `forward` replies.

## Appendix A. Amendments to accepted records, appended on 2026-10-19

**ADR 0116, Amendment (2026-10-19).** The capability of #897 is `HttpServer` (ADR 0123). The examples of rules 10 and 14, written with `Listener[accept, close]` and `accept`, read as `HttpServer[next, close]` and `next`, with `handle(exchange)` for `handle(connection)`, and without the `if error use unit` after `close()`, which cannot fail (a handling clause there is U3025, `spec/06 §3`); the trace of rule 14 names `host:HttpServer`, `HttpServer` and `next`. `HttpServer.next` joins the interruptible operations of rule 12, with `ServeError.Interrupted` and `ServeError.TimedOut` as its failures.

**ADR 0117, Amendment (2026-10-19).**
- Rule 3: the operation that ADR 0123 marks is `HttpServer.next`, which parks its task and is interruptible. `reply` and `close` do not park.
- Rule 2: a started task whose captures hold a reply right of ADR 0123 runs on the worker that owns the reply right's connection, instead of the next worker in turn; the choice is O(1), and the task stays pinned.
- Rule 8: grants of `HttpServer` (`serve:`) also carry limits, those of ADR 0123 rule 7, and "a limit on a grant other than `Http`" is U7002 only for the other capabilities.
- Rule 7 and the examples of rules 7 and 9: `Connection` and `Listener` read as `Exchange` and `HttpServer`, `accept` as `next`, and the `if error use unit` after `close()` is dropped, since `close` cannot fail (U3025, `spec/06 §3`).
- Rule 8 and implementation plan step 5: "ticket" there is the number that orders the waiters of a limit, not the `ReplyRight` of ADR 0123.

## Amendment (2026-11-02): the serve threads (slice S4, issue #1006)

- Status: Accepted by the project on 2026-11-02, with the decisions of its section F and of its last section. Drafted on 2026-10-25 for issue #1006 as `docs/plan/0123-s4-amendment-draft.md`, which now points here. The accepted text of this record above this amendment is unchanged.
- Design: `docs/plan/0123-s4-design.md`, whose §10.2 holds the soundness argument in full.

### Why

ADR 0123 rule 19 puts the server's work on the workers of ADR 0117, and has the worker that accepts a connection own it. Under ADR 0116's amendment of 2026-10-25, a precise `τ_conc`, that leaves two problems. The server's I/O then runs inside the turns of the workers, where its moments, set by the program's earlier hand-offs, shift the tasks beside every site, and the only label that covers it is P10's over every ancestor of a server operation, in effect `τ⁰` of the whole program. And which worker accepts depends on how busy each worker was, so the placement of request tasks carries the history of earlier code. This amendment moves the server to threads of its own, begins accepting only when the program first serves, places connections by a keyed hash of their addresses, and labels what remains.

### A. A decision on the request stream

**The stream's phase is input.** Question 4 made the request stream input, with its dependence on the server's timing out of model. This amendment decides how that reads for timing: the bytes peers send, and their timing relative to the program's own steps, are input, labeled `internal`. Two runs compared under T6 receive the same stream relative to their steps.

### B. Rule 19, native code

1. **Serve threads.** Dedicated serve threads serve the connections, `--serve-threads` of them (rule 20), and never the workers of ADR 0117 rule 2, which run tasks only. A serve thread runs no Una code. It has its own `kqueue` or `epoll`, owns the connections placed on it, and does their TLS, reading, parsing and writing.
2. **When accepting begins.** The serve threads and their pollers start before `main` (ADR 0116 P8, whose static condition gains an operation of `HttpServer` or a `[serve]` table). No listener is registered, and nothing is accepted, until the run's first server operation (a call of `next`, `reply` or `close`, a `start` that moves a reply right, or the start of the `[serve]` driver), which sets this once. Before it, a peer's connection waits in the kernel's backlog, the network's. A `close` that comes first never begins accepting.
3. **Accepting.** Each grant keeps one listening socket, without `SO_REUSEPORT`. Every listener is registered one-shot in one accept poller of the run, which is itself registered one-shot in the poller of one serve thread at a time, the one that holds the turn; that thread accepts at most 16 connections of each listener that fired, registers the listener again, guarded by a flag so that one thread re-arms it at a time, and passes the turn to a sleeping serve thread, else to another that is alive (a thread that dies while it holds the turn passes it on). A slot of `connections` is taken before each `accept`, so every accepted descriptor, one closed at once included, holds one. This replaces `EPOLLEXCLUSIVE` and `EV_DISPATCH`, and needs no new code with `unsafe`.
4. **Placement of connections.** A connection belongs to serve thread `H(listener, peer address, peer port) mod M`, with `M` the serve threads and `H` SipHash-2-4, with a domain tag, under a 128-bit key of the run from the operating system's random source. The key serves only this placement and the peer table's shards, under distinct tags, and appears in no trace, report, message or error. It replaces "the worker that accepts a connection owns it".
5. **Placement of tasks.** ADR 0123 Appendix A, amendment of ADR 0117 rule 2 ("a started task whose captures hold a reply right runs on the worker that owns the reply right's connection") is withdrawn. Every task is placed by ADR 0116 P6.
6. **The `[serve]` path.** A request is read on its serve thread, handled in a task on a worker, and written on its serve thread: two hand-offs between threads per request, as for the language form. "With no hand-off of the request between threads" no longer holds.
7. **What is shared, and the locks.** Rule 19's atomic counters stay, and a request's shares are atomics. The locks a request or a delivery may meet are declared:
   - the queue of requests waiting on each serve thread, under a mutex of that thread, held for O(1), taken by the thread and by `next`;
   - the list of tasks waiting in `next` for delivery, under a mutex per grant, taken only on the waiting path;
   - a connection's pending reply, under a mutex of that connection, uncontended except under reply pressure;
   - under reply pressure only, a mutex per grant that keeps the heaps choosing the slowest connections, taken by every reservation while pressure lasts;
   - the peer table, 64 shards of hash maps under mutexes per grant, locked at accept, close and expiry only.
8. **Reply pressure, bounded.** A pending reply is held in segments of at most 16 KiB, each dropped, and uncounted, as soon as it is written, so the counters of `replies` and `peer_replies` equal the reply bytes allocated. The thread whose reservation takes a grant over `replies` frees its victims' segments at once, before it allocates its own; the victims' serve threads only close the descriptors later. Rule 13's worst case gains: the replies being reserved at one moment, at most one per worker, and, with TLS, the buffers of `rustls`, bounded per connection. `reply` has no check point.
9. **Descriptors.** Before `main`, the run raises its soft limit, within the hard limit, to the sum of `connections`, plus one per listener, one for the accept poller, three per serve thread, 256 for the program's own `Http` requests, the `Files` helpers' open files (`--file-threads`, or 1 in the interpreter), and 64 for the rest of the runtime. Every socket of the program's `Http` counts against its 256, the I/O pool's fallback included.
10. **The `Costs` expectation.** ADR 0123 Costs expects the `[serve]` path "within the bar ... whose structure is that of a thread-per-core Rust server, less the shared counters". With two hand-offs per request it now reads: within the bar is the aim, to be confirmed by the measurement of section G; the hand-offs are the cost to measure, besides the shared counters.

### C. Rule 20, the interpreter and the flag

`--serve-threads <n>`, from 1 to 256, applies to `una run` and to executables. Its default is the number of cores in the interpreter, and half the cores, at least 1, in native code. `--workers` and its default do not change. Under `--virtual-time` both engines use one serve thread. A run under `--virtual-time` that holds a `serve:` grant without `--serve-script` is a usage error before `main`.

### D. Information flow

IF-S1. **The label of the serve threads, `L_srv`.** (Named so as not to collide with ADR 0116's IF8.)
- **Hand-off sites:** the calls of `next`, `reply` and `close`; each `start` whose captures can hold a `ReplyRight` or a function value; the driver of `[serve]`.
- **`sfx(c)`,** for a call site `c` in a body: `τ⁰` of the statements that can run at or after `c`: at each level of nesting, the statement that holds `c` and every statement after it in its block; a loop around `c` whole; the handling clauses of those statements and of every enclosing statement; the `on stop` block of every enclosing `for ever`; and everything these call.
- **`Suf*`** joins `sfx` along every edge of ADR 0116 P1 that leads to a node, from the roots: direct calls, `start` and the parts of constructs, calls through `Λ` and `uses of` parameters, operations through `K(c)` to every handler, and calls inside lambdas and handlers' operations.
- **The label:**

      L_srv = internal ⊔ ⨆ over hand-off sites s of ( sfx(s) ⊔ Suf*(node(s)) ⊔ τ_conc(s) ) ⊔ L_res

  `L_srv` joins `τ` of every timing site of a package whose program can perform an operation of `HttpServer`, and is `public` elsewhere. P11's `Suf`, once issue #1155 lands, MAY replace `sfx`.
- **Why it is sound.** Before the first hand-off, the serve threads hold no connection and do no work, so their state then is fixed, as P10's "every bucket starts full". From it on, they run no Una code, and what they do is a function of the request stream (input relative to the program's steps, section A, `internal`), of the program's hand-offs, and of timers the two arm. Every step between a hand-off and a later timing site is in the hand-off's suffix, or beside the site (`τ_conc`), or inside it (`τ⁰`). So the serve threads' work, and its moments relative to the site, are the same in two runs that differ only above the site's label.
- **The machine.** Given that, what remains is how the host shares its cores, caches and allocator between the serve threads and the workers. ADR 0116 P12 puts this out of model, as for any program whose timing other threads of the host share. P12 applies only because nothing is accepted before the first hand-off: no serve work exists whose moments set-up code could shift.
- **Placement and turns.** P6 places every task, and no server work runs inside a worker's turn, so P7 is unchanged.
- **Timers** of connections may fire up to 1 ms late where `epoll_wait` rounds its timeout; that phase is covered by `L_srv`.

**IF5, addition.** Which request a call of `next` receives depends on the timing of the serve threads and of every task that calls `next`, background tasks included, not only on the handlers. It is part of the request stream's dependence on the server's timing, out of model as Question 4 decided.

### E. Rule 22, Diagnostics, and `spec`

- **Rule 22.** The `out_of_model` list gains, for packages with a `serve:` grant or a `[serve]` table: "which request a call of `next` receives when several serve threads hold requests, which the timing of every task of the run and of the serve threads decides".
- **Diagnostics.** In ADR 0123's list of usage errors, "a descriptor limit below `connections` plus 64 that the run cannot raise" reads "a descriptor limit below the target of rule 19 (section B, item 9) that the run cannot raise"; the list gains "`--virtual-time` with a `serve:` grant and no `--serve-script`".
- **U6009** extends its scope: it is also reported at the start of a run that can serve when the host's random source gives no bytes for the placement key, before `main` and before any effect. `spec/13 §4` and its paragraph on U6009 in §6 change with it.
- **`spec/31 §2` rule 7, item 1** reads: "raises the run's limit of open descriptors to the target of `spec/31 §6`, within the hard limit; a run that cannot is a usage error that names the target", and `spec/31 §6` states the target of section B, item 9.

### F. Questions for the project, decided on 2026-11-02

1. **Dedicated serve threads (recommended), or the engine on the workers?** Decided: dedicated serve threads, as recommended, with the measurement of section G as the condition already written.
   - **Cost of dedicated threads:** two hand-offs between threads per request on every path, about a microsecond each at saturation, and about one and a half threads per core by default in native code; rule 19's `[serve]` path no longer stays on one core.
   - **Cost of the engine on the workers:** a label of every timing site at `τ⁰` of the whole program, since every ancestor of a server operation counts; in a program that reads `secret` data anywhere, every `within` and `first of` becomes `secret`, and rule 15's `start ... within` and rule 16's `[serve] within` are refused in any server that touches `secret` data.
2. **The cost of `L_srv` to programmers** (with dedicated threads).
   - Every timing site of a server is at least `internal`, as IF7 makes request tasks already.
   - When code that can run after a server operation reads `secret` data (a handler, code after the accept loop, a handling clause or an `on stop` block), `L_srv` is `secret`: inside the server's `tasks` statement this adds nothing to P2, which already gives its sites the statement's `κ`; outside it, a later construct with a `within` or a `first of` becomes `secret` too.
   - Code that runs only before the first server operation, such as set-up that loads keys or computes on `secret` data, does not raise it.
   - Peers that connect before the first server operation wait in the kernel's backlog, at most `min(connections, 65535)` per grant, until the program serves.

   Recommended: accept, with P11's `Suf` (#1155) as the later relaxation. Decided: accepted, as recommended.

### G. The measurement condition

Before ADR 0123's performance step, the project measures, on a quiet machine in release form: connections accepted per second and the cost of each listener event (one wake and a message to the next thread), CPU time per request at saturation, the 50th and 99th percentile latency under uniform and skewed handler costs, and memory per idle connection, for (i) this design, (ii) a prototype with the engine on the workers and per-worker accept, and (iii) the `hyper` on `tokio` baseline of ADR 0123 Costs. When (i) is above the parity bar against (iii), CPU time per request at saturation above 1.1 times, and (ii) is within it, a further amendment weighs that gap against the label that (ii) needs.

### Decisions of the implementation (added with #1375 and #1376, decided on acceptance)

Items 1 to 4 are confirmed as written, with the notes each carries; item 10 states the fault policy the project chose.

1. **Half-close ends the exchange.** A peer's end of stream while its request waits for a reply ends the exchange and the reply is dropped (rule 14); a client that sends a request and shuts down its sending side loses the reply. Confirmed: it frees a slot early and leaves no half-open connections to hold. S5 (#1007) states in `spec/31` and in the guide that a client keeps its sending side open until the reply, and revisits the choice only for a real use.
2. **No byte, no `408`.** A connection that sends no byte within `read` is closed without an answer; with any byte, `408`. Confirmed (RFC 9110 §15.5.9 allows it, and it answers no scanner or speculative connection). Whether a connection gets `408` or a silent close depends on timing, so the timing oracles of S5 (#1007) cover the choice under `L_srv`.
3. **`Host` ignores the port.** The port of `Host` is read for syntax and not compared with the grant's. Confirmed: authorization never depends on the port of `Host`, which the client chooses. If a program ever needs the port, S5 exposes it to the program; no `421` is answered.
4. **The registry lock.** Besides atomics, a reply takes one short lock of one of 16 registry shards (to find the connections with a reply pending in O(c) when pressure begins) and its connection's outbox lock. "No lock outside pressure" in the design means no shared lock between serve threads and the program on the request path, except these two. Confirmed; it is changed only if the measurement of section G shows the lock in the profile.
5. **Shares not implemented.** `waiting`, `/once` and `delivered` have no per-peer share (rule 13 has none for them): about 16 well-behaved addresses can fill `waiting` at the defaults. Whether S5 or an amendment adds them is open.
6. **Accept turn.** The accept poller is watched by one serve thread at a time and the turn passes round, instead of every thread watching it (which woke all threads on every listener event).
7. **Stop races.** A reply serialized before a stop and delivered after it closes its connection, and may lack `Connection: close`.
8. **Compaction latency.** Rebuilding the pressure heaps is O(live) under the grant's pressure lock: a latency spike for one reservation, amortized O(1) per push, not a worst-case bound.
9. **A serve thread that ends** by a panic outside a connection's step gives up its inbox, closes every connection it owned and gives back what they held (slots, peers' places, requests, exchanges, replies), hands the peers' expiries it was timing to a live thread, hands on the turn to accept, and registers again every listener whose event its last wait returned and whose burst did not end; what a connection held is given back as far as its state records it. A message sent to it afterwards is handled by its sender (a connection handed to it closes and gives back what it took; a peer's expiry goes to a live thread). A new connection whose keyed owner ended goes to the next live serve thread in the ring; with none left it is refused at accept.
10. **Faults: contain per connection, stop the run in three cases** (decided 2026-11-02; to be implemented by #1391, open). Today's code, from #1376, contains every panic listed below, the callbacks' included, and prints each one through the panic hook; the policy that follows is what #1391 changes it to. A panic in a connection's step outside the thread's timing wheel and readiness registrations, in the hand-off of an accepted connection, or in a peer's expiry, is caught where it happens and counted: it closes that connection at most, never a serve thread, so an input that makes the host panic costs one connection and never a share of the server's capacity. The run stops instead, with a defined error, when:
    - a callback the program registered (`on_ready`, `on_slot_freed`) panics: the fault is the program's, and the program must see it;
    - a panic happens while a lock of a grant's shared state is held, after which that state may be inconsistent and is never served from again;
    - the contained faults of the run pass a fixed threshold, the same for every run (#1391 fixes its value; 64 is proposed).

    A panic inside the wheel or the readiness ends the thread (item 9). Contained steps do not print through the panic hook, so faults cannot flood the log. At S5 (#1007) the program reads the count of contained faults through `faults()`, and each contained fault is a trace event; until then they are counted and not shown. #1391 also closes the windows in which a contained fault leaves a slot or a peer's place held: a fault in a peer's admission after its place was taken, a fault in the hand-off before the connection reaches its owner's inbox, and a panic inside closing code. It states the error of a stop, and amends `spec/31 §6` rule 13 with this policy. Each callback is registered once per run; a second registration is a defined error (`AlreadyRegistered`) in every build and changes nothing, so S5 registers its callback once, not once per `next`.

## Amendment (2026-11-05): the native move at `start`

- Status: Accepted by the project on 2026-11-05. Drafted for issue #1571 (slice S5b) as `docs/plan/0123-s5b-native-move-amendment-draft.md`, which now points here. The wording of the cost and of the order was corrected afterwards to match the code (the pass runs after the copy succeeds, and native code walks every captured value); the corrected wording was confirmed by the project's decision of 2026-11-06. The accepted text of this record above this amendment is unchanged.
- Concerns: rule 4 item 2 ("A task's reply rights are found by the walk that copies its captures, so the walk costs nothing in native code") and the Costs row "Move reply rights at `start`" ("native: none beyond the copy of the captures (ADR 0113)").

### Why

Both engines must move the rights that a `start`'s captures reach in one order: each marked capture in the order of its name, each value depth first, left to right, an object that several values share at its first meeting (`spec/31 §1` rule 3 item 2). That order decides the order of the started task's held list, and so the order of its auto-answers and of their `unanswered` events (rule 4 item 5).

The copy of the captures that ADR 0113 makes in native code (`values::export`) walks its objects in another order: right to left, children before their parent, each step announced and checked against the memory budget before it allocates. Two ways to find the rights in the copy were considered:

- **Reorder the copy.** The copy's order is where a copy is refused (U6003) and what keeps it deterministic. Reordering it would move the memory-refusal points of every program, servers or not, for a cost that only servers need. Rejected.
- **Record the rights where the copy meets them.** In the copy's order, an object that several values share is reached at its last left-to-right meeting, not its first, so the order of moves, and with it the order of auto-answers and their events, would differ from the interpreter's whenever a right is shared. Rejected.

So native code finds the rights with a pass of its own over the values of the closure that `start` copies, in the order above. The order of the steps is: the copy of ADR 0113, then the pass, then the task is queued. The pass runs after the copy succeeds and before the task is queued, so a refused copy moves no right to a task that never exists.

### Amendment

The Costs row "Move reply rights at `start`" reads, for native code: one extra pass, O(objects that the task's captured values reach), the same order as the copy itself, only at a `start` with at least one capture that can hold a `ReplyRight`; none otherwise. When any capture is marked, native code hands over every capture and walks all of them; the interpreter walks only the marked ones. Both engines move the same rights in the same order, because an unmarked capture holds no right. Rule 4 item 2's "so the walk costs nothing in native code" reads: native code finds the rights in a pass of its own over the captured values, in the order of the interpreter's walk, after the copy of ADR 0113 succeeds and before the task is queued; the copy's order and refusal points do not change. The order of moves and the result are the same in both engines.

### Bounds

- Time: each object that the captured values reach is looked at once (a set of the objects seen), so the pass is O(objects reached by the task's captures), the copy's own order of cost; a scaling test bounds it (`values::tests::the_walk_of_start_is_linear_in_the_values_it_reaches`).
- Space: the pass's stack and its set of the objects seen, O(objects reached by the task's captures), released before the task is queued.
- A `start` whose captures cannot hold a reply right, as Core IR marks it (`rights` empty), makes no pass at all.

## Amendment (2026-11-06): what slice S5 settles, and the count of contained faults

- Status: Accepted, by the project's decision of 2026-11-06 on the draft `docs/plan/0123-s5-amendment-draft.md`, which now points here. The accepted text of this record above this amendment is unchanged; where the two differ, this amendment governs.
- Issues: #1007 (the checks of slice S5) and #1571 (the operations in both engines). Design: `docs/plan/0123-s5-design.md`.

### Why

Implementing rules 0 to 5 and the Information flow of this record in the checker met one conflict with another normative text and several points the record leaves open. Each is settled here the conservative way. Sections A to H and J state what the implementation does; section I declares what a later part of #1571 shows to the program.

### A. The field `query` of `Request`

Rule 1 declares `Request` with a field `query: internal List[Param]`. `spec/01` reserves `query` in every edition ("reserved for v0.4 (platform)", U0103), so a field of that name cannot be declared, and `request.query` could never be read. The reservation is older than this record, which states that it "adds no reserved or contextual keyword"; the clash went unnoticed.

The field is `params: internal List[Param]`, with the same meaning: the query's parameters, decoded, in order. Rule 1 and its examples read `params` for `query` where they name the field; the trace's reduced form ("the values of query parameters, in `query` and in `target`") reads "in `params` and in `target`". The word `query` stays reserved.

### B. What `next` accepts

Rule 1 declares `accepts internal` on `reply` and, in its prose, `public` only on `close`; it writes no `accepts` on `next`. Without one, a call of `next` under any program counter would be accepted. Whether a task takes a request off the queue decides which task holds its reply right and takes a `/once` slot that other tasks' deliveries wait for, so a call under a `secret` program counter would let `secret` data decide what other tasks observe.

`next` accepts `internal`. An accept loop runs under a `public` program counter (IF1), and a request task under an `internal` one (IF7), so no server of this record's examples changes; only a call of `next` under a `secret` condition is U4003.

### C. The function values of IF6

IF6 checks a `start` whose captures can hold a `ReplyRight`, by their types, "or include a function value, which may have captured one". Read alone, the second clause would check every `start` that captures a function value in every package of edition `0.4`, servers or not, and refuse ones under a `secret` program counter that hold no reply right and never could.

The clause applies in a package that imports `HttpServer`, `Exchange` or `ReplyRight` from `std.host`, the only names through which a value of the package can hold a reply right (a `ReplyRight` comes only from `next`, and later from `test_exchange`, whose types name them). The first clause, a capture whose type can hold a `ReplyRight`, applies everywhere.

### D. U3033 through handlers

Rule 5 forbids an operation of `HttpServer` in the effect set of a branch. A branch that calls an operation of another capability, `@orders.find()`, runs whatever handler serves that binding, and a handler's parameters may hold a reply right, so two branches could still answer one right in an order that timing decides.

An operation, in a branch, of a capability that has a handler in the package whose `uses` hold `HttpServer` (or such a capability, to a fixpoint) is U3033, whatever handler serves it at run time.

### E. `L_srv` with the preview's terms

The amendment of 2026-11-02 (IF-S1) defines `L_srv` with `sfx(s)`, the statements that can run after a hand-off site, and `Suf*`, its closure along the call graph. The checker computes `sfx(s) ⊔ Suf*(node(s))` as `τ⁰` of the bodies of every ancestor of the function that holds the site, the rule `L_lim` already uses (`spec/27 §9` rule 5), with the preview's `τ_conc`. It is sound and coarser: code that runs only before the first server operation raises the label too, which section F.2 of that amendment says it does not. Under the preview's `τ_conc`, a server of the language form has `L_srv = ⊤` in any case, since its `next` stands in a `tasks` block.

This stays until #1155 (P11's `Suf`) or a statement-level `sfx` lands, as `spec/31 §1` rule 5 item 7 states it: a preview over-approximation that MAY become more precise and MUST NOT become stricter.

### F. Runs before the runtime

Slice S5 lands in parts: the checks (#1007), then the operations in both engines (#1571), with the timing oracles that `spec/31 §6` requires before the serve threads meet a program. Between them a program can name `HttpServer` but not run it. `una run` and `una build` of a program whose entry function uses `HttpServer` are a usage error, exit code 2, before anything runs (`spec/31 §1` rule 10); `una check` and `una test`, whose tests install handlers with `given`, work as for any capability.

### G. Two readings added after review

- **`uses of` parameters in branches (rule 5).** A function that calls its `uses of` parameter in a branch runs code its callers choose, which the branch cannot see: in a program that imports a name of the server, such a call, or the passing on of the parameter, is U3033, failing closed.
- **Type parameters (IF6).** A capture of a type parameter's type counts as one that can hold a reply right only in a package that imports `HttpServer`, `Exchange` or `ReplyRight`, as for function values (section C).

### H. The reduced trace form

Rule 1 states the reduced form of the events of `next` and `reply` and that "their values are redacted by their labels, as every event's are". Four details are settled (`spec/31 §1` rule 7, one implementation for both engines):

1. A header is a credential by its name in any letter case: a reply's `Set-Cookie` is one, as a request's `cookie` is (requests carry names in lower case already). A header whose name is not a token, such as `"set-cookie "` or a name with a tab or a control byte, counts as a credential too, failing closed: the host refuses to send such a reply (`BadReply`), but its event still holds the value, and no spelling of a credential's name may escape the reduction.
2. `<credential>` and `{"length": n, "hmac": "<hex>"}` are written bare in the value's Una form, as `<reply-right>` and `<redacted:level>` are, so they cannot be mistaken for a text of that content.
3. In `target`, which stays a quoted text, each value of a query part `name=value`, split at its first `=`, is replaced by the body form of the value as received, before its percent-decoding; a part without `=` is a name and stays, as names stay in `params`. In `params` the body form is of the decoded value.
4. The fail-closed rendering that the first part of the slice used until the reduced form landed, every argument of the host's `HttpServer` written as an `internal` value whatever its label, gives way to rule 1's own reading: redaction by labels, the reduced form beneath it. Since every field of `Request` and `Reply` is declared `internal`, a run's trace still shows no field of either; a `Reply` built from `public` data is written `Reply(status: <redacted:internal>, headers: <redacted:internal>, body: <redacted:internal>)` instead of `<redacted:internal>`, which shows nothing but its static label. A run without the engine keeps the fail-closed rendering.

The key of the HMAC is held by the engine of the run: all zeros under a script, and 32 bytes from the host's random source for the serve threads of a live run, where a host that gives none stops the run from starting, as for the placement key.

### I. `faults()` and the event of a contained fault

Decision 10 of the amendment of 2026-11-02 says that at S5 the program reads the count of contained faults through `faults()` and that each contained fault is a trace event, and the fault policy places both at the one point every contained fault passes. Neither text declared `faults()`, its type, its label, where it lives, or the event's form. They are:

1. `faults` is a fourth operation of `HttpServer`: `op faults() -> internal Int`, the run's count of contained faults, `accepts internal`, no failure, never parking. It is `internal` because the count depends on what peers send and when (IF5). It is an operation rather than a function, because it reads the host's state, so it is an event and is refused in a branch of `together` by U3033 as the other operations are. A `serve:` grant allows it with the others. The per-grant counts stay in the event of item 2.
2. Each contained fault appends a host event with no task and no operation, `{"seq": n, "outcome": {"kind": "fault", "site": "step" | "accept" | "peer-expiry" | "abandon", "listener": "<grant>"}}`, written at the program's next server operation after it (the events of serve threads are not in any task's order), before that operation's own event, so that a trace stays a function of a script under `--virtual-time`.
3. Both land with a later part of #1571, in both engines. Until then, contained faults stay counted and unseen, as `spec/31 §6` rule 13 item 5 states.

### J. The root's abort and `next`

An abort of the root does not lower the run's cancellation entry: the root unwinds through its `tasks` statements, which cancel their tasks. A task waiting in `next` when the root aborts ends the operation `interrupted`, with its `cancelled` event, as a cancelled one does; only an abort by a started task ends a `next` with the run's abort. Both engines agree (`tests/serve_runs/aborted_root_ends_a_waiter`).

## Amendment (2026-11-06): the depth of a request task in IF6

- Status: Accepted, by the project's decision of 2026-11-06 on the draft `docs/plan/0123-if6-depth-amendment-draft.md`, which now points here. The accepted text of this record above this amendment is unchanged; where the two differ, this amendment governs.
- Issue: #1674, found by the security hunt of the serve engine (finding F1, Medium, both engines). Implemented by PR #1678, with `spec/31 §1` rule 5 item 6.

### Why

IF6 checks the auto-answer of a `start` that moves reply rights "under `pc ⊔ τ(e)` when it has `within`, and under `pc` otherwise, against `internal`", and says that "a reply inside `e` already meets the same bound under ADR 0117's timing floor, so this rejects only a `start` whose expression never replies".

That leaves out how the task ends. ADR 0117 rule 7 makes a started task that runs past `--task-depth` end alone, and this record's rule 4 item 5 answers its rights `500`. Whether a peer gets the program's reply or the host's `500` then depends on how deep `e` recursed. A `reply` reached after a recursion on `secret` data runs under a `public` or `internal` program counter, so it is accepted, and the recursion only computes `secret` values, so it is legal too. The hunt's probe, a task that recursed `key - body.len` deep, recovered a secret integer exactly by a binary search on the body's length, in 14 requests, in both engines.

`spec/07 §1` puts termination and stack exhaustion out of model, under termination-insensitive noninterference, which limits such a leak to about one bit per run because the run ends. A task that ends alone does not end the run: every request turns termination into a status the peer reads, as often as it likes. The model's reason for leaving termination out does not hold here.

### A. The rule

IF6 gains: a `start e` that moves reply rights is also U4003 at the `start` when both:

1. **its task can push frames without a bound its code shows:** what `e` runs, along the call graph of ADR 0116 P1 with an edge from each policy to its `via` function, reaches a cycle (a function, a handler, a `via` or a lambda that can call itself again), a call of a function value or of a `uses of` parameter, or code whose calls the checker does not hold (a standard module other than the prelude, a dependency not checked with the package);
2. **it can hold data above `internal`:** the program counter at the `start`, what `e` reads from outside, binds or receives from an operation, or what any node it reaches declares in its signature, binds, receives or, in a lambda, captures. A symbolic label `ℓ_T` counts as above `internal` in the `start`'s own data, since a `start` in a generic body takes the values of its type parameters from callers outside the task, and in the lambdas of the package when the task can be handed a function value made outside it; it does not count in a generic function the task reaches, whose type parameters take what the task passes.

The last sentence of IF6 ("a reply inside `e` already meets the same bound ...") reads: "A reply inside `e` meets the bound of the program counter already; the depth of its task is bounded by the rule above."

**Why it is sound.** A frame is pushed only by a call of Una code. If item 2 fails, nothing the task runs holds data above `internal`, so whatever decides its depth is at most `internal`, which the answer accepts. If item 1 fails, the task's calls form no cycle, call no function value and stay in code the checker holds, so its depth is bounded by the longest chain of calls that code shows. The prelude's functions are host code in both engines, which push no frame of Una code but through a function argument, itself a call of a function value; this assumes that host code does not exhaust a task's stack on data of its own.

**Type parameters.** A rule that dropped every symbolic label, on the ground that callers count the values of a type parameter, holds for a generic function the task calls, whose callers are the task's code, and is false for a `start` in a generic body: the caller that instantiates `T` with `secret Int` is outside the task. The rule keeps the symbols of the `start`'s own data, and those of `Λ` when a lambda made outside the task can reach it.

**What it does not cover yet.** With item 1 false the task still ends alone when the operator sets `--task-depth` below that longest chain, and which chain runs may depend on data above `internal`. The default `--task-depth` is 1000. Issue #1681 proposes that the engines refuse such a setting.

**What it refuses.** The rule does not ask whether the data decides the recursion: a task that recurses on the request's length and also reads a `secret` key for a policy is refused. That is the price of a rule simple enough to be clearly sound and linear. Issue #1682 records a precise rule (what decides each call) as a later refinement, which MAY only accept more.

**Why not a runtime fix.** The host cannot make a depth-aborted task's `500` look like the reply the task would have sent, so a uniform auto-answer does not close the channel. Leaving it out of model would weaken a public guarantee.

### B. A correction found on the way

The checker ran the default of `if error use` under the program counter from before the clause, not under the label that decides the failure, so its calls and effects were checked as if they ran whatever the failure, a gap of `spec/07 §3`. The implementation raises the program counter before the clause's context is taken. No accepted text changes for it.

### C. Out of scope: other effects after a depth abort

A task that ends alone also skips every effect it would have performed after the abort, which a public sink may see, and its `aborted` event and stderr line name it. Under `spec/07 §1` that is termination, out of model, as it is for an abort of the run. It is not observable by the peer through the reply right, which this amendment covers. Issue #1677 records it for a decision of the model.

### Cost

One graph of the package, one search for its cycles and two closures over its strongly connected components, O(V + E), at the first `start` that moves reply rights; then O(size of the seed of `e`) lookups per `start`. Recording what a node holds costs O(open sites + log n) per binding and per operation.

## Amendment (2026-11-06): the kind `TestRequestError`

- Status: Accepted, by the project's decision of 2026-11-06 on the draft `docs/plan/0123-s7-amendment-draft.md`, which now points here. The accepted text of this record above this amendment is unchanged.
- Issue: #1009 (slice S7). Precedent: ADR 0117, "Amendment (2026-10-19): what slice S3 settles", item 7, which settled `Deadline = Expired` as `= | Expired` for the same reason.

Rule 21's `pub kind TestRequestError = Refused` is declared `pub kind TestRequestError = | Refused`: a sum kind of one variant, since `= Refused` would name an alias of a type `Refused` (`spec/02 §2`, `alias_body`), which does not exist, and `fails` names a sum kind (`spec/06`, U2002 otherwise). `test_request` fails with `Refused` where the host would answer `400`, as rule 21 states. The variant and its meaning do not change (`spec/31 §8` rule 2).

## Amendment (2026-11-08, Proposed): a per-peer share of `waiting`, `/once` and `delivered`

- Status: Proposed, for the project to accept. Nothing in it is implemented. The accepted text of this record above this amendment is unchanged; once accepted, this amendment governs where the two differ.
- Issue: #1784, the residual of decision 5 of the amendment of 2026-11-02 ("`waiting`, `/once` and `delivered` have no per-peer share").

### Why

Rule 7 gives a peer a share of `connections` (`peer`), of `handshakes`, of `buffers` (`peer_buffers`) and of `replies` (`peer_replies`), but none of the three budgets that requests hold after they are read: `waiting` (read, not delivered), `/once` (delivered, not answered) and `delivered` (their bytes). Today what one peer holds of them is bounded only by its connections, since a connection has at most one request between the end of its reading and its reply (`spec/31 §3` rule 8):

| Budget, default | Bound on one peer today | Peers that fill it at the defaults |
| --- | --- | --- |
| `waiting` = 1024 | `peer` = 64 requests | 16 |
| `/once` = 256 | `peer` = 64 exchanges | 4 |
| `delivered` = 256 MiB | 64 × (`line` + `header` + `body`) ≈ 65 MiB | 4 |

A body's reservation in `peer_buffers` (1 MiB at the defaults) ends at delivery (rule 13 item 2), so after delivery it no longer limits the peer: one peer can hold 64 delivered requests of about 1 MiB each.

**The attack.** Take the rate-limited proxy of the Examples (`600/minute`, so 10 deliveries a second, `answer` 30 s). An attacker with 16 IPv4 addresses opens 64 keep-alive connections from each and keeps one small `GET` in flight on every connection, sending the next as soon as a reply or a `503` arrives. After about 100 seconds of the rate, every place of `waiting` holds one of its requests, and stays held: each place it loses to a delivery or to `answer` it takes back at once. Every request of every other peer then finds `waiting` full and gets `503` with `Retry-After: 1` (rule 14) at once, for as long as the attack lasts. With a slow handler (an upstream that takes seconds) instead of a rate, 4 addresses hold all 256 `/once` slots, or all 256 MiB of `delivered` with bodies near `body`, and every other request waits up to `answer` and gets `503`. And an operator who raises `connections` to 16,384 gets `peer` = 1024 by default, so one address fills `waiting` alone. Rule 7 already says that global limits are levers ("The shares make this cost many addresses, not one"); for these three, the cost is a handful of addresses, or one.

### Options

Every option keys peers as rule 12 does (`PeerKey` in `crates/una-http/src/serve/peer.rs`): an IPv4 address; the /64 prefix of an IPv6 address; an IPv4-mapped IPv6 address as its IPv4 address. Each keeps its counters in the peer's entry of the peer table, which lives while the peer has an open connection, and an open connection outlives every request it holds, so no option adds an entry or a lock at accept. Two caveats hold for all of them, as they hold for `peer` today:

- **NAT.** Many clients behind one carrier-grade NAT or one proxy are one peer, and share one share. A server behind a proxy sets the shares to `none`, as it sets `peer`, `peer_buffers` and `peer_replies` (Question 7).
- **IPv6.** A peer is a /64. An attacker with a /48 holds 65,536 peers, and a share per /64 does not slow it down; neither does `peer`. A second level of shares per /48 is left out here (see the recommendation).

#### Option A. A fixed share, taken when a request is read in full

Two items join rule 7:

| Item | Meaning | Default | Range |
| --- | --- | --- | --- |
| `peer_pending=N` | requests of one peer read in full and not yet answered: waiting for delivery or in an open exchange | `max(16, waiting / 64)` | 1 to 2^31 − 1, or `none` |
| `peer_delivered=M` | the bytes, head and body, of those requests, in MiB | `max(1, delivered / 64)`, raised to the bytes of one request at `line`, `header` and `body` | 1 to 65,536, or `none` |

- **Where it is taken.** At the end of reading a request (`complete` in `thread.rs`), beside the place in `waiting`, with the same `add_below` on two atomics of the peer's entry; a failure of a later check gives back the earlier ones. A request over either share is answered as when `waiting` is full, `503` with `Retry-After: 1`, at once, and takes no place. A declared `Content-Length` that cannot fit `peer_delivered` even alone is refused when the head is read, as #1675 refuses one that cannot fit `delivered`.
- **Where it is given back.** When the exchange ends (reply, `answer`, the peer leaving, a stop, an auto-answer), or when the waiting request is refused or dropped: the same points that give back `/once` and `delivered` today.
- **Why at reading, not at delivery.** A share checked at delivery would stop the front of a serve thread's queue on its own peer's share, and every request behind it, of every peer, would wait (head-of-line blocking); skipping it would make delivery a scan. Taking the share before the request is queued leaves delivery as it is: first come, first served per serve thread, from the rotating cursor (`spec/31 §6` rule 5), O(1) expected.
- **What it bounds.** At the defaults a peer holds at most 16 waiting or open requests, and 4 MiB of their bytes (a request of `body` = 1024 KiB, with `line` and `header`, fits). Filling `waiting` takes 64 peers (four times today), `/once` 16 (four times), `delivered` 64 (sixteen times); and an operator's larger `connections` no longer lets one peer fill `waiting`.
- **Cost.** Time O(1) per request, two atomic additions and two subtractions, no lock. Space 16 bytes per peer entry (two `AtomicU64`), at most `connections + handshakes` entries (`spec/31 §6` rule 2 item 5).

#### Option B. A fair share that follows the active peers

A peer may hold `max(floor, budget / active)` of each budget, where `active` is the number of peers that hold any request; or, in the variant like rule 13's reply pressure, nothing is refused until a budget is full, and then a request of a peer under its fair share displaces the newest waiting request of the peer that holds the most (which gets `503`).

- **What it bounds.** A lone peer may use a whole budget; under contention each active peer gets an equal part. It adapts to load where option A does not.
- **Cost.** The first form: time O(1) per request (one more atomic, the count of active peers, raised and lowered as a peer's count leaves and reaches 0), space 16 bytes per peer plus one counter per grant. The variant: a heap of peers by what they hold, under a mutex per grant, O(log p) per change over p active peers while the budget is full, space about 32 bytes per active peer for the heap's entry and its index; a request's place must also be removable from its queue, which today's queues mark dead and compact.
- **Against it.** A peer's refusal then depends on how many other peers are active, a count the peer reads from its own `503`s, finer than today's "a budget is full". That is a new channel between peers, outside this record's model of information flow (it is between peers, not between labels of the program) but contrary to the aim of the shares, which is that one peer's service does not reveal or depend on another's beyond the global limits. The variant's displacement also refuses requests that were already accepted, a behavior no other limit of this record has.

#### Option C. A per-peer rate of requests

A token bucket per peer, `peer_rate`, as rule 10 keeps for handshakes: GCRA, one timestamp per peer, charged when a request is read in full; a request without a token is answered `503` with `Retry-After: 1`.

- **What it bounds.** What a peer holds of `waiting` only indirectly, at most its rate times the time a request stays, up to `answer` (30 s): at 16 requests a second, 480 places. It does not bound `/once` or `delivered` under a slow handler, which a slow request holds without any rate, and it refuses the bursts that browsers make (several requests at a page load) more often than an occupancy share does.
- **Cost.** Time O(1) per request, one compare-and-swap; space 8 bytes per peer.

#### Comparison

| | A, fixed share | B, fair share | C, per-peer rate |
| --- | --- | --- | --- |
| Time per request | O(1), atomics | O(1); variant O(log p) under a lock | O(1), one CAS |
| Space per peer | 16 bytes | 16 bytes; variant +32 | 8 bytes |
| Bounds `waiting`, `/once`, `delivered` | all three, directly | all three, adaptively | `waiting`, indirectly |
| New channel between peers | none | the count of active peers | none |
| Delivery path changed | no | variant: yes | no |

### Information flow, the report and the oracles (option A)

- **`L_srv` (IF-S1).** The shares are state of the serve threads that changes only at the end of reading a request, at delivery and at the end of an exchange, each an event of the request stream or of a hand-off. The soundness argument of IF-S1 is unchanged: the serve threads' work stays a function of the request stream (input, `internal`), of the program's hand-offs and of their timers. No label changes.
- **IF5.** A refusal by a share depends on the peer's own requests and on how fast the program answers them, the same dependence as a refusal by a full `waiting`, already out of the model (Question 4). IF5's list "full `waiting`, `buffers` or `connections`" reads "full `waiting`, `buffers`, `connections`, `peer_pending` or `peer_delivered`". A fixed share depends on no other peer, so it creates no channel between peers; it removes part of one, since another peer can no longer fill `waiting` alone.
- **Timing.** The checks are two atomic additions whose cost does not depend on any value, and the answer is the same `503` as for a full `waiting`, sent at the same point, so a refusal by a share and one by the global budget cannot be told apart by their timing or bytes.
- **Determinism.** Nothing is keyed or random; under a script (`--virtual-time`, `--serve-script`) the refusals are a function of the script.
- **The trace.** The host's refusals are not events of the program, so no event changes. Which requests `next` receives changes only as a full `waiting` already changes it.
- **The report (rule 22).** Each grant's entry lists `peer_pending` and `peer_delivered` with their values, as for every limit. The warnings gain "a grant with `peer_pending=none` or `peer_delivered=none`". The `out_of_model` list is unchanged.
- **The oracles.** The five oracles of `spec/31 §6` time a reply to a probe, the owner of a connection, or a delivery under a rate; none is meant to depend on a budget's share, but every one of them, like every test of the toolchain, sends from one peer, loopback. The slice of tests checks that each keeps fewer requests of that peer in flight than `peer_pending` and, where an oracle or a test fills a global budget from loopback on purpose, sets `peer_pending=none` and `peer_delivered=none` in its grant, so that it still tests the global limit.

### Tests that would prove it

All in host code shared by both engines (`crates/una-http/src/serve`), with the serve-run cases of `tests/serve_runs` run in both engines:

- **One peer.** One address, 64 connections, a handler that does not answer: the 17th complete request is `503` with `Retry-After: 1` at once; after one reply, the next request is accepted; the global `waiting`, `/once` and `delivered` stay within the peer's share. A request whose bytes exceed `peer_delivered` alone is refused at its head, before its body.
- **Many peers.** 64 addresses at their share fill `waiting` exactly; a 65th peer's request gets the global `503`; when one peer's request is answered, another peer's request takes the place. With 63 peers at their share, a new peer is served while they hold theirs.
- **Slow peers.** Peers that never read their replies, peers that send their bodies a byte at a time, and peers that leave while waiting or after delivery: every share is given back at the end of each exchange, and after every connection closes the grant and every peer entry are quiet (`is_quiet`, extended to the peer counters).
- **Keys.** Two addresses of one /64 share a share; two /64s do not; `::ffff:a.b.c.d` shares with `a.b.c.d`.
- **Races.** Many threads taking and giving back shares of one peer never pass the share and end at zero, as `gate.rs`'s `racing_threads_never_pass_the_limit` does for handshakes.
- **Scaling.** The time per admission stays flat at 1,000, 10,000 and 100,000 peers (a test that fails on growth linear in the peers).
- **Grants.** Parsing, defaults, ranges and `none` of both items; U7002 in `una.toml` and the usage error for `--grant` on a malformed value, as rule 6 lists them.

### Recommendation and the default

**Option A**, with the defaults above. It bounds all three budgets directly, costs O(1) and 16 bytes per peer, leaves the delivery path and its order untouched, and creates no channel between peers. Option B's adaptivity is not worth a channel between peers in a record whose rule is the most secure option; option C does not bound `/once` or `delivered`. If the project does not answer, the default is option A as written here, defaults included. Left for a later record: a share per IPv6 /48 above the share per /64, a share of the grant's rate (option C as an addition), and fair queuing of delivery across peers (a round robin of per-peer queues on each serve thread, O(1) amortized per delivery, about 48 bytes per active peer and serve thread), which would also bound how long one peer's requests delay another's.

### Implementation slices

Each is under 250 changed lines and one area; 1 and 4 can go in parallel, 2 after 1, 3 after 2.

1. **Grant items** (`crates/una-http`, the grant parser and `Limits`): `peer_pending` and `peer_delivered`, defaults, ranges, `none`, the errors of rule 6, the report's warning; unit tests.
2. **The accounting** (`serve/peer.rs`, `serve/grant.rs`, `serve/thread.rs`): the two counters of `Peer`, taking them in `complete`, giving them back wherever `/once`, `delivered` or a `waiting` place is given back, the refusal at a head that cannot fit; unit and race tests, `is_quiet` with the peers.
3. **Adversarial runs** (`crates/una-http/src/serve/tests.rs`, `tests/serve_runs`): one peer, many peers, slow peers, keys, scaling, both engines; the grants of loopback tests that fill a global budget get `none`.
4. **Text** (`docs/spec/31-http-servers.md` §2 and §6 rules 4, 5, 7 and 12; the guide; a changelog fragment): the items, where they are taken and given back, the answers, the costs; rule 13's worst case per peer.
