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
   4. The header section's lines, its trailers' included, count against `headers`, and its bytes, each line with its CRLF and the empty line that ends it included, against `header`. Over either, `431`, refused before the bytes over the limit are kept.
4. **Framing, the defense against smuggling.** In this order:
   1. `Content-Length` and `Transfer-Encoding` together: `400`.
   2. `Transfer-Encoding` in an `HTTP/1.0` request: `400`.
   3. Two `Transfer-Encoding` headers: `400`.
   4. A `Transfer-Encoding` whose whole value is not `chunked`, in any letter case: `400` when it is malformed (empty, an empty element, an element that is not a bare token, parameters included) or names `chunked` twice; else `501`, since it names a coding the host does not implement.
   5. Two `Content-Length` headers, even with equal values: `400`.
   6. A `Content-Length` value that is not 1 to 19 ASCII digits alone, with no sign, whitespace or list: `400`. A value above `body`: `413`, before any byte of the body is read.
   7. Neither header: an empty body.
   8. **Chunked bodies.** Each chunk size MUST be 1 to 16 hex digits alone, with no whitespace, sign or extension; anything else is `400`. This departs from RFC 9112 §7.1.1, which asks a recipient to ignore unknown chunk extensions, because lenient reading there has been a route for smuggling. The line after a chunk's data MUST be empty. A chunk whose size would take the body past `body` is `413` at its size line, before any of its data is kept. Trailers are header lines of rule 3, read and dropped; their lines and bytes count against what the header section left of `headers` and `header`, not a fresh allowance.
5. **Splits.** Reading is incremental, and a request split at any byte boundary reads exactly as in one piece. Each byte is examined a bounded number of times, so reading is linear in the bytes read.

## 4. The target

1. **Origin-form.** The target MUST start with `/`. The asterisk-form and the authority-form are `400`.
2. **Characters.** The path holds only `/` and the characters of an RFC 3986 `pchar`; the query, after the first `?`, holds those and `?`. Each `%` MUST be followed by two hex digits. Anything else is `400`: a `#`, a space, `\`, `|`, `{`, `"`, `^`, a backquote, `<`, `>`, `[`, `]`, and any byte above 0x7E. RFC 3986 does not allow them unencoded.
3. **Segments.** The path after its first `/` is split on `/`, and each segment is percent-decoded alone, so an encoded `/` never makes a new segment. A segment that decodes to `.` or `..`, or that holds `/`, `\`, NUL or another control character (below 0x20, or 0x7F), or that is not UTF-8 after decoding, is `400`. `/` gives `[""]`, and `/a//b` gives `["a", "", "b"]`. Forms that other servers read as special, such as `..;` and the double-encoded `%252e%252e`, are ordinary segments.
4. **Query.** The query is split on `&`, and empty parts are skipped. Each part is `name=value`, split at its first `=`, or `name` with the value `""`. A `+` decodes to a space, then percent-decoding applies. A name or value that is not UTF-8 after decoding is `400`.

## 5. Writing a reply

The checks and the bytes of a reply (ADR 0123 rule 13) join this section with the server's writer.
