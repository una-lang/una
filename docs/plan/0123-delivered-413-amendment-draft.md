# Draft: ADR 0123, Amendment: `413` for a request that can never fit `delivered` (issue #1675)

- Status: Not accepted. By its decision of 2026-11-06 the project did not accept this amendment; it stays a draft, and ADR 0123 rules 10 and 14 stand as written: a request that can never fit `delivered` is answered `503` with `Retry-After: 1` at once, as `spec/31 §6` rule 5 states. Dated 2026-11-06.
- Origin: finding F2 of the hunt of #1392.

## Why

A grant may set `delivered` below the bytes of one request: `delivered` starts at 1 MiB and `body` goes up to 1 GiB, and the bytes charged to `delivered` are the request as read, head, body and the framing of a chunked body. Such a request can never be delivered. Before #1675 it held the front of its serve thread's queue until `answer`, and every request behind it on that thread got `503`.

#1675 answers it at once, without a place among the `waiting` requests, so the others are delivered. The answer it gives is the one ADR 0123 rule 14 lists for a request that "waits longer than `answer` for ... room in `delivered`": `503` with `Retry-After: 1`. A request that can never fit is certain to wait past `answer`, so the implementation gives that answer earlier, and the status does not change. A declared `Content-Length` is checked when the head is read, before the body; a chunked body when the request is read in full.

## The point to decide

`503` with `Retry-After: 1` tells the client to retry a request that can never succeed under this grant. `413 Content Too Large` (RFC 9110 §15.5.14) says the request itself is too large, which is the case.

**Proposed:** rule 14 reads, in its list of `503` answers, "room in `delivered`" as room for a request that could fit `delivered`; and rule 10's `413` also covers "a request whose bytes as read exceed the grant's `delivered`, answered at once, when its head is read for a declared `Content-Length` and when it is read in full for a chunked body, with no `Retry-After` and with `Connection: close`". `spec/31 §6` rule 5 and the table of §6 rule 7 would follow.

## When `413` is better, and its costs

- **Better for clients.** A client that honors `Retry-After` retries every second until it gives up; with `413` it stops at once and can report the cause.
- **Deterministic.** The answer depends only on the request's own bytes and a static limit of the grant, never on load or on how fast the program answers, so it reveals nothing about other exchanges. The same holds for the `503` given at once, which is why the interim reading is sound.
- **No guarantee weakens.** Nothing is delivered either way; the change is the status and the absence of `Retry-After`.
- **Cost.** One more case for `413` in rule 10, where it was only "a body over `body`"; a client cannot tell from the status alone whether `body` or `delivered` refused it, which does not matter to it. No code cost: the status is one constant in `Core::complete` and `Core::admit_head` of `crates/una-http/src/serve/thread.rs`, and the test `a_request_larger_than_delivered_is_answered_503_at_once` changes its expected status.

## Alternative not proposed

Refusing the grant when `delivered < body + line + header`, a usage error before `main`. It does not close the case, since chunk framing makes a request's bytes larger than `body + line + header`, and it would refuse grants that serve most requests well today.
