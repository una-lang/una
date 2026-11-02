# 31. HTTP servers

Status: Normative, edition 0.4 preview (ADR 0123).

A server answers HTTP/1.1 requests through the host capability `HttpServer`, granted by address with limits. The rules of ADR 0123 join this chapter as they land, each with its tests. Until the capability lands, no program can name it, and the rules below bind the host code that will read, write and serve for it (`crates/una-http`), which its tests check.

**Editions.** The rules of this chapter apply only to packages of edition `0.4` (`spec/13 §7`). Earlier editions do not change, and the client of `spec/24` does not change.

## 1. The capability

The declarations of `HttpServer`, `Request`, `Reply`, `Exchange` and reply rights (ADR 0123 rules 1 to 5) join this section when the capability lands.

## 2. Grants

A `serve:` grant lets a run accept connections on one address and port, with limits that every server has whether its grant writes them or not (ADR 0123 rules 6 to 9). The limits named in §3, `line`, `headers`, `header` and `body`, are items of this section.

1. **The text form.** In `--grant` (`spec/13 §5`): `serve:<scheme>://<address>:<port>`, then items as suffixes `;<item>`.
   - `<scheme>` is `http` or `https`, in lower case.
   - `<address>` is an IPv4 address in dotted-quad form, or an IPv6 address in brackets, written in its canonical form (RFC 5952: lower case, the longest run of zero groups compressed, no leading zeros), so one address has one spelling. `0.0.0.0` and `[::]` mean every interface. A host name, a zone, an IPv4-mapped IPv6 address (`[::ffff:127.0.0.1]`, written as the IPv4 address instead) and a multicast address are refused: nothing resolves a grant, and a socket bound with `IPV6_V6ONLY` receives no IPv4 connection. A link-local IPv6 address (`fe80::/10`) is unsupported: without a zone it cannot be bound, so the run fails closed with the usage error of rule 7.
   - `<port>` is from 1 to 65535, always written, with no leading zero. Nothing follows it but items.
   - A number in an item is decimal digits with no sign and no leading zero (`0` alone where the range allows it).
2. **Items.** Each item has a default, and each is given at most once; one rate at most, of any period, since a grant keeps one token bucket.

   | Item | Meaning | Default | Range |
   | --- | --- | --- | --- |
   | `N/once` | exchanges delivered and not yet answered | 256 | 1 to 2^31 − 1 |
   | `N/second`, `N/minute`, `N/hour` | exchanges delivered per period, a token bucket | none | 1 to 2^31 − 1 |
   | `waiting=N` | requests read and waiting to be delivered | 1024 | 0 to 2^31 − 1 |
   | `connections=N` | open connections | 1024 | 1 to 1,048,576 |
   | `peer=N` | open connections from one peer | `max(16, connections / 16)` | 1 to 1,048,576, or `none` |
   | `handshakes=N` | TLS handshakes started per second; one peer may start `max(4, N / 16)` | 512 | 1 to 2^31 − 1 |
   | `requests=N` | requests on one connection | 1000 | 1 to 2^31 − 1 |
   | `line=K` | the request line, in KiB | 8 | 1 to 64 |
   | `headers=N` | header lines of a request, trailers included | 100 | 1 to 1000 |
   | `header=K` | the header section, trailers included, in KiB | 16 | 1 to 256 |
   | `body=K` | a request's body, in KiB | 1024 | 0 to 1,048,576 |
   | `buffers=M` | bytes of request bodies held for the grant, in MiB | 64 | 1 to 65,536 |
   | `peer_buffers=M` | the share of `buffers` of one peer, in MiB | `max(1, buffers / 64)` | 1 to 65,536, or `none` |
   | `replies=M` | bytes of replies not yet written, in MiB | 64 | 1 to 65,536 |
   | `peer_replies=M` | the share of `replies` of one peer under pressure, in MiB | `max(1, replies / 64)` | 1 to 65,536, or `none` |
   | `delivered=M` | bytes of the requests of open exchanges, in MiB | 256 | 1 to 65,536 |
   | `idle=S` | seconds a connection waits for a request's first byte | 5 | 1 to 3600 |
   | `read=S` | seconds to read a whole request | 20 | 1 to 3600 |
   | `write=S` | seconds to write a whole reply | 30 | 1 to 3600 |
   | `answer=S` | seconds from the end of a request to the start of its reply | 30 | 1 to 86,400 |
   | `hosts=a,b` | the names `Host` may carry (rule 5) | IP literals and `localhost` | names, or `*` |
   | `certificate=F`, `key=F` | the certificate chain and the private key, PEM files | none | `https` only, and required there |
   | `tls12` | also accept TLS 1.2 | off | `https` only |

   A derived default is computed from the values the grant gives, so `connections=4096` makes the default `peer` 256. Sizes and counts are compared in unsigned 64-bit arithmetic that never wraps. Every default is settled: a later release may only tighten one.
3. **The `ita.una` form.** Under `run grants`, `serve "<scheme>://<address>:<port>"`, then one item per line below it, each the item of rule 2 it names:

   ```ebnf
   serve_grant = "serve" text_lit NEWLINE [ INDENT serve_item { serve_item } DEDENT ] ;
   serve_item  = ( "certificate" text_lit | "key" text_lit | "tls12"
                 | "at" "most" int_lit ( "at" "once" | "per" ( "second" | "minute" | "hour" ) )
                 | ( "waiting" | "connections" | "requests" | "headers" ) "at" "most" int_lit
                 | "peer" ( "at" "most" int_lit | "none" )
                 | "handshakes" "at" "most" int_lit "per" "second"
                 | ( "line" | "header" | "body" ) "at" "most" int_lit "KiB"
                 | ( "buffers" | "replies" | "delivered" ) "at" "most" int_lit "MiB"
                 | ( "peer_buffers" | "peer_replies" ) ( "at" "most" int_lit "MiB" | "none" )
                 | ( "idle" | "read" | "write" | "answer" ) "within" int_lit ( "second" | "seconds" )
                 | "hosts" text_lit { "," text_lit } ) NEWLINE ;
   ```

   ```una
   run
     grants
       serve "https://0.0.0.0:8443"
         certificate "tls/chain.pem"
         key "tls/key.pem"
         at most 256 at once
         at most 6000 per minute
         body at most 64 KiB
         answer within 10 seconds
         hosts "shop.example", "*.shop.example"
   ```

   A unit is written where the table has one, and only that unit, so no line is read in a unit it does not name.
4. **Errors and editions.**
   - In `ita.una`, each of these is U7008 at its line: a line of another form, an item given twice or a second rate, a value out of range, `certificate`, `key` or `tls12` on an `http` grant, an empty path or a path with `;`, and a `hosts` line of rule 5's errors; at the grant's line, an invalid scheme, address or port, an `https` grant without `certificate` or `key`, and an overlap (rule 6).
   - With `--grant`, each is a usage error, exit code 2, before anything runs; so is a `serve:` grant for a single file or a package of an edition before `0.4`.
   - In `una.toml`, a `serve:` grant is U7002 in every edition: before `0.4` there is no server, and from `0.4` the grants are read from `ita.una`.
   - A grant allows all three operations of `HttpServer`.
5. **`hosts`.** Each name is a registered name of letters, digits, `-`, `_` and `.`, with no empty label and a last label that is neither all digits nor starts with `0x`, as `Host` writes one (§3 rule 5); one trailing `.` is removed and the name is compared in ASCII lower case. A name MAY start with `*.`, which stands for exactly one label (`spec/24 §2`); `*` alone accepts every name of valid syntax and is the only item of its list. An IP literal, a port, an empty name and a name given twice are errors. With the `Host` read by §3 rule 5:
   - without `hosts`, on every grant whatever its address, the name MUST be `localhost` or an IP literal, else `421`;
   - with `hosts`, the name MUST equal a listed name or match a `*.` name, else `421`; an IP literal matches no name.
6. **Overlap.** Each address and port is granted once, and a wildcard address may not share its port with another grant of its family, whatever the schemes. IPv6 sockets are bound with `IPV6_V6ONLY`, so `[::]:p` and `0.0.0.0:p` do not overlap. In `ita.una` the later of two grants that overlap is U7008; among the grants of a run, manifest and flags together, an overlap is a usage error.
7. **Before `main`.** After the checks of `spec/13 §5` and before `main` starts, the host, in this order:
   1. raises the run's limit of open descriptors to the sum of `connections` over the grants, plus 64, within the hard limit; a run that cannot is a usage error;
   2. for each `https` grant, opens the key file and then the certificate file, once each, checks each open descriptor, and reads each from that descriptor (rule 8);
   3. binds one listening socket per grant, without `SO_REUSEPORT`, with `IPV6_V6ONLY` on IPv6, so no other process shares its connections; an address that cannot be bound is a usage error that names it. The socket sets `SO_REUSEADDR`, so a server restarts while connections of its previous run are in `TIME_WAIT`; a second listener on the same address and port is refused. On macOS and the BSDs, another local process can still bind a more specific address on the same port while a grant binds the wildcard (`0.0.0.0` or `[::]`), and then receives the connections to that address. The defense is a grant that names a specific address rather than the wildcard.
8. **TLS files.**
   - **Opening.** Each file is opened once, following symbolic links, without blocking on a FIFO, and checked on the open descriptor with `fstat`, never by its path, so a file swapped between the check and the read is impossible. Each MUST be a regular file of at most 1 MiB. Relative paths resolve as other grants' paths do (`spec/13 §5`).
   - **The key file** MUST also be owned by the run's effective user or by root, have no write bit for its group and no bit for others (`st_mode & 0o027 == 0`), and have its group read bit only when its group is the run's effective group or a supplementary group. A key owned by another user is refused even when the run is root's.
   - **Contents.** The certificate file holds a chain of `CERTIFICATE` blocks, leaf first; the key file a private key in PKCS#8, SEC1 or PKCS#1 form. A file that cannot be read, or that holds no such block, is a usage error.
   - **The key stays in the host.** No operation returns it, and it is in no value, trace, diagnostic or report.
   - **The protocol, pinned.** TLS 1.3 only, unless the grant says `tls12`, which adds TLS 1.2 with the extended master secret required. No renegotiation, no early data, no stateless session tickets (resumption through a bounded in-memory cache only), no key log, ALPN `http/1.1` only, and no client certificates.
   - **Pending.** The check that the key matches the certificate, and the TLS server itself, wait for the decision on the project's TLS provider (#1059); until then a run with an `https` grant loads and checks its files and binds, as every grant does, and nothing accepts on any listener.
   - **Other platforms.** An `https` grant is a usage error on a platform other than macOS and Linux, until a later record defines the check of a file's access list.

## 3. Reading a request

The host refuses every request that two HTTP implementations might read differently, and never repairs one. Each refusal answers the status shown, with `Connection: close`, and closes the connection after it, so the host never looks for where a broken request ends.

1. **Request line.** It MUST be `method SP target SP version CRLF`, with single spaces and nothing before the method; an empty line before it is refused, not skipped. A line that is not three parts split by single spaces, a method that is not a token (RFC 9110 §5.6.2), or a target with a byte outside the printable ASCII range 0x21 to 0x7E is `400`. A version other than `HTTP/1.1` and `HTTP/1.0` is `505`. A token that is not one of `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE` and `OPTIONS`, in upper case, is `501`, `CONNECT` and `TRACE` included. A request line longer than `line`, its CRLF included, is `414`, refused as soon as it cannot fit and before its bytes are kept.
2. **Line ends.** Every line MUST end with CRLF. A bare LF, or a CR not followed at once by LF, anywhere in the request line, the header section or the chunked framing, is `400`.
3. **Header lines.**
   1. A header line is a name that is a token, followed at once by `:`, then a value. A line that starts with a space or a tab (obsolete folding) is `400`; whitespace before the colon is `400`.
   2. A value is trimmed of spaces and tabs at both ends. A byte below 0x20 other than tab, or 0x7F, in a value is `400`. A value that is not UTF-8 is `400`, and is never repaired.
   3. A `Connection` header is a list of tokens separated by commas, with optional spaces and tabs around each. Every token, compared in any letter case, MUST be `close`, `keep-alive`, or a header that belongs to the connection: `connection`, `keep-alive`, `proxy-connection`, `te`, `trailer`, `transfer-encoding`, `upgrade` and `content-length`. Any other token, or an empty element, is `400`, so a peer can never make the host remove a header that a front proxy added, or `host`.
   4. A header name that differs from `host`, `content-encoding`, `expect` or a header of rule 3.3 only by `_` in place of `-`, such as `Transfer_Encoding`, is `400`: a proxy that maps one to the other would read a framing or a host that the server does not.
   5. The header section's lines, its trailers' included, count against `headers`, and its bytes, each line with its CRLF and the empty line that ends it included, against `header`. Over either, `431`, refused before the bytes over the limit are kept.
4. **Framing, the defense against smuggling.** In this order:
   1. `Content-Length` and `Transfer-Encoding` together: `400`.
   2. `Transfer-Encoding` in an `HTTP/1.0` request: `400`.
   3. Two `Transfer-Encoding` headers: `400`.
   4. A `Transfer-Encoding` whose whole value is not `chunked`, in any letter case: `400` when it is malformed (empty, an empty element, an element that is not a bare token, parameters included) or names `chunked` twice; else `501`, since it names a coding the host does not implement.
   5. Two `Content-Length` headers, even with equal values: `400`.
   6. A `Content-Length` value that is not 1 to 19 ASCII digits alone, with no sign, whitespace or list: `400`. A value above `body`: `413`, before any byte of the body is read.
   7. Neither header: an empty body.
   8. **Chunked bodies.** Each chunk size MUST be 1 to 16 hex digits alone, with no whitespace, sign or extension; anything else is `400`. This departs from RFC 9112 §7.1.1, which asks a recipient to ignore unknown chunk extensions, because lenient reading there has been a route for smuggling. The line after a chunk's data MUST be empty. A chunk whose size would take the body past `body` is `413` at its size line, before any of its data is kept. Trailers are header lines of rule 3, read and dropped; their lines and bytes count against what the header section left of `headers` and `header`, not a fresh allowance.
5. **`Host`.** A request MUST carry exactly one `Host`, `HTTP/1.0` requests included, else `400`. Its value is a host and an optional `:` with a port of 1 to 5 digits, at most 65535. The host is a registered name of letters, digits, `-`, `_` and `.`, with no empty label; an IPv4 address in dotted-quad form; or an IPv6 address in brackets, without a zone. A registered name whose last label is all digits, or starts with `0x`, and which is not an IPv4 address, such as `127.1`, is `400`, since resolvers read such names as addresses in different ways. Anything else, such as an empty value, `a/b@c`, or an empty port, is `400`. One trailing `.` of a registered name is removed, and the name is compared in ASCII lower case; the program sees the value as sent. Which names a grant accepts (`421`) is the grant's (§2 rule 5).
6. **Encodings and expectations.** A `Content-Encoding` other than `identity`, in any letter case, is `415`. In an `HTTP/1.1` request, `Expect: 100-continue`, in any letter case, asks the host to send `100 Continue` once it admits a body; any other expectation is `417`. In an `HTTP/1.0` request, `Expect` is ignored.
7. **Order.** The request line is checked when it ends. The whole head is checked when its empty line arrives, before any byte of the body is read, in this order: the framing of rule 4, `Connection`, `Host`, the target of §4, `Content-Encoding`, `Expect`.
8. **The body** is read to the end its framing gives, and no further: the bytes after it belong to the next request, which is read only after the reply to this one is written. A body that is not valid UTF-8 is `400`.
9. **What the program sees.** The headers of rule 3.3 that belong to the connection are removed. The other names are in ASCII lower case, in the order received, with their trimmed values. The connection closes after the reply when `Connection` lists `close`, or when the request is `HTTP/1.0`.
10. **Splits.** Reading is incremental, and a request split at any byte boundary reads exactly as in one piece. Each byte is examined a bounded number of times, so reading is linear in the bytes read.

## 4. The target

1. **Forms.** The target MUST start with `/` (origin-form), or be in absolute-form, `http://` or `https://` in any letter case followed by an authority and an optional path and query. An absolute-form target is accepted only when its scheme is the grant's and its authority equals the `Host` value in ASCII letter case; it is then rewritten to the origin-form of its path and query, `/` for an empty path, which the program sees as `target`. Else it is `400`. The asterisk-form and the authority-form are `400`.
2. **Characters.** The path holds only `/` and the characters of an RFC 3986 `pchar`; the query, after the first `?`, holds those and `?`. Each `%` MUST be followed by two hex digits. Anything else is `400`: a `#`, a space, `\`, `|`, `{`, `"`, `^`, a backquote, `<`, `>`, `[`, `]`, and any byte above 0x7E. RFC 3986 does not allow them unencoded.
3. **Segments.** The path after its first `/` is split on `/`, and each segment is percent-decoded alone, so an encoded `/` never makes a new segment. A segment that decodes to `.` or `..`, or that holds `/`, `\`, NUL or another control character (below 0x20, or 0x7F), or that is not UTF-8 after decoding, is `400`. `/` gives `[""]`, and `/a//b` gives `["a", "", "b"]`. Forms that other servers read as special, such as `..;` and the double-encoded `%252e%252e`, are ordinary segments.
4. **Query.** The query is split on `&`, and empty parts are skipped. Each part is `name=value`, split at its first `=`, or `name` with the value `""`. A `+` decodes to a space, then percent-decoding applies. A name or value that is not UTF-8 after decoding is `400`.

## 5. Writing a reply

1. **Checks.** Before any byte is written, a reply MUST have a status from 200 to 599; no body on a `204` or a `304`; header names that are tokens, in any case; header values with no CR, LF, NUL or other control character but tab, and no 0x7F; none of `Connection`, `Content-Length`, `Date`, `Keep-Alive`, `Proxy-Connection`, `TE`, `Trailer`, `Transfer-Encoding` and `Upgrade`, in any letter case; at most 100 headers, and at most 64 KiB of them, each counted as `name: value` with its CRLF. Otherwise `reply` fails with `BadReply`, nothing is sent, and the reply right stays held.
2. **The bytes.** `HTTP/1.1 <status> <reason>`, with the reason phrase of RFC 9110 §15 (and of RFC 6585 for `429` and `431`), or an empty one for any other status; then `Date` as an IMF-fixdate; `Content-Length` with the length of the body given, except on a `204` or a `304`; the program's headers, in order, as given; `content-type: text/plain; charset=utf-8` when the reply names no content type and carries a body; `Connection: close` when the connection closes after the reply; the empty line; and the body, unless the request was `HEAD` or the status is `204` or `304`. A `HEAD` reply keeps the `Content-Length` of the body given and gets no default content type.
3. **The host's own answers** (refusals, `421`, `503`, `500`) have an empty body, `Content-Length: 0`, `Retry-After: 1` where ADR 0123 rule 14 asks for it, and `Connection: close` when the connection closes, as it does after every refusal of §3 and §4. `100 Continue` is the status line and the empty line alone.
4. **Cost.** Checking and writing a reply of r bytes is O(r), into one buffer reserved once.

## 6. Connections and serve threads

The host code in `crates/una-http/src/serve` is the same in both engines and runs on **serve threads**: threads that never run program code and whose work is bounded and linear in the bytes and events they handle (ADR 0123 rules 12 to 14 and 17 to 20; `docs/plan/0123-s4-design.md`). No program can name `HttpServer` yet, so until it lands (ADR 0123 step 5) a run holds its listeners and accepts nothing; the engine is exercised by its own tests, with a consumer written in Rust in place of the program.

1. **Serve threads.** `una run` and executables take `--serve-threads <n>`, from 1 to 256, by default the number of the machine's cores (`spec/27 §3`). A value that is not a number, or is out of range, is a usage error, exit code 2, before `main` starts. A run that serves nothing accepts the flag and uses none. Each serve thread owns its connections, its timers and an inbox, and has a readiness queue of its own (`kqueue` or `epoll`) and a pair of sockets that wake it. Only plain owned data crosses between threads: a connection, the bytes of a request, a serialized reply.
2. **Accepting.**
   1. Accepting begins at the run's first server operation. Before it, no listener is registered, and a peer's connection waits in the kernel's backlog.
   2. Each grant keeps the one listening socket that §2 rule 7 binds, without `SO_REUSEPORT`. Every listener is registered, one-shot, in one accept poller, which is itself registered, one-shot, in the readiness queue of one serve thread at a time, the one that holds the turn (thread 0 first). That thread accepts at most 16 connections of each listener that fired, registers the listener again, and passes the turn to the next serve thread by a message. So exactly one thread accepts from a listener at a time, a listener event wakes one thread whatever their number, and a burst of connections is spread over the threads in turn.
   3. A slot of `connections` is taken before `accept` is called, so every descriptor the host accepts, one closed at once included, holds one. At `connections` the host does not register the listener again until a connection of the grant closes, and the kernel's backlog holds the rest. `ECONNABORTED` and `EINTR` are skipped; any other error of `accept` stops accepting on that listener for 100 ms.
   4. A connection is then checked in this order: `peer`, and for an `https` grant the peer's and then the grant's handshake rates (rule 10). A connection over any of them is closed at once, before any byte is read. Else it is placed on the serve thread named by SipHash-2-4 of the listener, the peer's address and the peer's port, under a key of 128 bits drawn from the operating system's random source for the run and shown nowhere; the place does not depend on the order of accepts, on which thread accepted, or on how busy any thread is.
   5. A peer is an IPv4 address, or the /64 prefix of an IPv6 address; an IPv4-mapped IPv6 address is its IPv4 address. The host keeps the peers with open connections, or with a handshake bucket that is not full, in a table of 64 shards: at most `connections + handshakes` entries.
3. **A connection** is in one of four states, with at most one timer at a time (§2 rule 2: `idle`, `read`, `write` and `answer`; every timer is a total and trickling bytes never extends it):
   1. *Reading.* The first request of a connection is read within `read` of the accept. A later request waits `idle` for its first byte, then `read` for the rest. A request not read in full within `read` is answered `408` and the connection drains and closes (rule 8); a connection that sent no byte of it is closed without an answer. A connection idle past `idle` is closed.
   2. *Waiting.* From the end of the request until the reply is handed over, within `answer`: after `answer` an undelivered request, or a delivered exchange that was not answered, is answered `503` with `Retry-After: 1`, and a reply that comes later is discarded. The host reads no more than the head reserve while it waits, so a peer that leaves is seen at once, and its exchange ends with its reply discarded.
   3. *Writing.* The reply is written within `write`, and the host reads nothing more from the connection meanwhile. Then the connection reads its next request, or closes when the request said `Connection: close`, when it was `HTTP/1.0`, when the connection reached `requests`, or when a host stop is under way.
   4. *Draining* after a refusal (rule 8).
4. **Bytes of requests, in shares.** The head of a request, `line` and `header` bytes, is counted in `connections` at the accept and waits for no shared budget. A body is admitted before its first byte is read: a declared `Content-Length` reserves its whole length, a chunked body reserves 64 KiB before each step, in the grant's `buffers` and the peer's `peer_buffers`; a reservation that fits neither is answered `503` with `Retry-After: 1` at once, and nothing else waits. `100 Continue` is sent only after the admission. The reservation is released when the request is delivered or refused.
5. **Delivery.** A request read in full takes a place among the `waiting` requests (`503` with `Retry-After: 1` when none is free), and is delivered when the next exchange fits the grant's `delivered` bytes, its `/once` slots and its rate, in this order, each undone if a later one fails. Requests are delivered first come, first served among those one serve thread read for one grant; across threads the queues are visited from a rotating cursor, so no queue waits behind another indefinitely. The wait is bounded by `answer`. The rate is a token bucket kept as one timestamp per grant, updated by compare-and-swap, with the interval rounded up to a whole nanosecond and never to zero; it holds `N` tokens.
6. **Replies, in shares.** A reply is checked and serialized once (§5), reserved on the grant's `replies` and the peer's share, cut into segments of at most 16 KiB, and handed to the connection's owner; handing it over never waits on a peer and never fails for the reservation. Each segment is dropped, and its bytes given back, as soon as its last byte is written. When a reservation leaves the grant over `replies`, the same call, before it allocates, closes connections until the grant is back under: the one with the most bytes left of the peer furthest over its `peer_replies` share; when no peer is over its share, the one with the most bytes left of all; ties to the connection accepted first. A closed connection's reply is dropped at once and is never written, which the program never sees. A single reply larger than `replies` is kept only while the grant has no other reply pending, else its connection is closed at once. The pending bytes of a grant are at most `replies` plus the replies being reserved at that moment.
7. **The host's own answers.** Each has an empty body and `Content-Length: 0`; `Retry-After: 1` is written where shown; `Connection: close` always.

   | Status | When | `Retry-After` |
   | --- | --- | --- |
   | `400`, `413`, `414`, `415`, `417`, `431`, `501`, `505` | a refusal of §3 and §4 | no |
   | `421` | `Host` is not allowed (§2 rule 5) | no |
   | `408` | a request is not read in full within `read` | no |
   | `503` | `waiting` is full; a body does not fit its shares; a request waits past `answer`; an exchange is not answered within `answer`; a host stop finds a request undelivered or being read; the hard stop; the task holding the reply right is cancelled | yes |
   | `500` | the reply right is dropped without a reply | no |

8. **Closing after a refusal.** After an answer that closes the connection, the host shuts down its sending side, reads and drops what the peer still sends, at most 64 KiB or for one second, and then closes, so that the peer reads the status instead of a reset. The connection keeps its slot and its place among the peers until it closes.
9. **Stopping.** At a host stop the host closes every listener at once and closes idle connections; requests undelivered or still being read are answered `503`; delivered exchanges drain, each reply carrying `Connection: close`; a reply serialized just before the stop may lack the header, and its connection closes after it all the same. At the hard stop, each open exchange is answered `503` in one non-blocking write, and every connection closes, without waiting on a peer. A reply to an exchange that the hard stop, `answer` or a departed peer ended returns as if it had been sent.
10. **Handshakes.** For an `https` grant, `handshakes` per second per grant and `max(4, handshakes / 16)` per second per peer are token buckets, each full at the start and holding one second of tokens, charged at the accept; at most half of the serve threads, rounded up, are inside a handshake step at a time. Until the TLS server lands (§2 rule 8), a run with an `https` grant does not start serving: the host fails closed instead of serving plain bytes on an `https` address.
11. **Descriptors.** The run's limit of open descriptors is raised before `main` to the sum of `connections` over the grants, plus one listener per grant, one for the accept poller, three for each serve thread (its readiness queue and its pair of wake sockets), 256 sockets for the program's own `Http` requests, the helpers of `Files`, and 64 for `Console`, the reactor and the runtime; a run that cannot is a usage error that names the sum. The cap on the sockets of the program's `Http` requests is then the soft limit less everything but those 256.
12. **Costs.** Accepting, queueing and delivering a request, reserving and releasing a share, taking a token, arming and cancelling a timer, and closing are O(1) expected; reading b bytes is O(b); a reply of r bytes costs O(r) to check and serialize and O(log c) per connection closed for pressure over c connections with a reply pending; a host stop is O(connections + queued requests).
13. **Faults.** A panic of the host in a connection's step outside the serve thread's timers and readiness registrations, in the hand-off of an accepted connection, in a peer's expiry, or in a callback the program's side registered, closes that connection at most (at accept, the connection is closed and gives back its slot and its peer's place); no serve thread ends by it. A panic inside the thread's timers or readiness registrations, which every connection of the thread shares, or anywhere else in the thread's loop, ends the thread. A serve thread that ends closes every connection it owned and gives back what they held as far as their state records it (rules 2, 4, 5 and 6; a panic inside the giving back itself leaves the rest held), hands on the turn to accept and the peers' expiries it was timing, and registers again the listener of a burst of accepting it was in; a message for it is handled by its sender; a new connection whose keyed owner ended is placed on the next serve thread in the ring that has not ended. Contained faults are counted but not yet shown to the program (step 5).

**Block for the capability.** The serve threads decide observable timing: `408` against a silent close, `idle` against `read`, the order of delivery, and the placement of connections. ADR 0123 step 5 (#1007) MUST NOT wire these engines to a program before the `L_srv` labels of the S4 design (`docs/plan/0123-s4-design.md` §10) and their timing oracles exist.
