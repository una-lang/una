# 31. HTTP servers

Status: Normative, edition 0.4 preview (ADR 0123).

A server answers HTTP/1.1 requests through the host capability `HttpServer`, granted by address with limits. The rules of ADR 0123 join this chapter as they land, each with its tests. Until the capability lands, no program can name it, and the rules below bind the host code that will read and write for it (`crates/una-http`), which its unit tests check.

**Editions.** The rules of this chapter apply only to packages of edition `0.4` (`spec/13 §7`). Earlier editions do not change, and the client of `spec/24` does not change.

## 1. The capability

The declarations of `HttpServer`, `Request`, `Reply`, `Exchange` and reply rights (ADR 0123 rules 1 to 5) join this section when the capability lands.

## 2. Grants

The `serve:` grant and its limits (ADR 0123 rules 6 to 9) join this section when grants land. The limits named below, `line`, `headers`, `header` and `body`, are the grant's items of ADR 0123 rule 7.

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
5. **`Host`.** A request MUST carry exactly one `Host`, `HTTP/1.0` requests included, else `400`. Its value is a host and an optional `:` with a port of 1 to 5 digits, at most 65535. The host is a registered name of letters, digits, `-`, `_` and `.`, with no empty label; an IPv4 address in dotted-quad form; or an IPv6 address in brackets, without a zone. A registered name whose last label is all digits, or starts with `0x`, and which is not an IPv4 address, such as `127.1`, is `400`, since resolvers read such names as addresses in different ways. Anything else, such as an empty value, `a/b@c`, or an empty port, is `400`. One trailing `.` of a registered name is removed, and the name is compared in ASCII lower case; the program sees the value as sent. Which names a grant accepts (`421`) is the grant's (ADR 0123 rule 8).
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
