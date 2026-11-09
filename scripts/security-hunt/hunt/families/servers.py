"""Family N: servers (ADR 0123, `spec/31`, #1011), edition 0.4.

Each program is one server in the language form (`spec/31 §1` rule 6) and a script of
connections (`spec/31 §8` rule 6) that stands for its peers under virtual time: the run binds no
listener and touches no network. `hunt servers` (`hunt/serving.py`) runs it with `--serve-script`
and judges what the peers received, `--serve-out`, against the expectation the generator wrote
(`Program.serve`).

Malformed requests (N1 to N4). The server answers every request it is given `200` with the body
`seen <target>`. Each program sends one request, malformed by one rule of `spec/31 §3` or §4 or a
well-formed twin of it, followed on the same connection by a second request, `GET /behind`, which
a peer that smuggles would want read:
- **a refusal** must be the host's own answer alone, with the rule's status, `Content-Length: 0`
  and `Connection: close`, its side shut, and the request behind it never answered, so the program
  never sees either (§3, §5 rule 3);
- **a twin** must be answered `200`, the request behind it too unless the first closes the
  connection (`HTTP/1.0`).
The same bytes are sent again on a second connection, split at random byte boundaries over
several instants, and the peer must receive the same bytes (§3 rule 10); a third connection then
sends a valid request, which must be answered, so no malformed request ends the server.

N1 the request line and line ends (§3 rules 1 and 2), N2 header lines, encodings and
expectations (rules 3 and 6), N3 framing, the defense against smuggling (rule 4, and rule 8's
UTF-8 body), N4 `Host` and the target, path confusion (rule 5, §4).

The family is not part of the release gate's fixed run (seed 767): its operators are in
`SERVER_OPERATORS`, apart from `OPERATORS`, which the gate runs; `hunt gen --ops N` and
`hunt repro --op N1` reach them.
"""

import base64
import json

from ..itaty import lock_text
from ..packages import package_files
from ..program import OPEN, Program, rng_for
from ..una import header, join_decls

MOD = "srv"
EDITION = "0.4"
EDITIONS = (EDITION,)
# The grant's address, as `listener` writes it, and its `serve` spec: no `hosts=`, so `Host`
# accepts `localhost` and IP literals only (`spec/31 §2` rule 5), and the default limits.
LISTENER = "127.0.0.1:8080"
GRANT = 'serve "http://127.0.0.1:8080"'
# The default limits the cases reach (ADR 0123 rule 7): `line` 8 KiB, `headers` 100 lines,
# `header` 16 KiB, `body` 1024 KiB.
LINE_LIMIT = 8 * 1024
HEADERS_LIMIT = 100
HEADER_LIMIT = 16 * 1024
BODY_LIMIT = 1024 * 1024

BEHIND = b"GET /behind HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
CONTROL = b"GET /control HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
SEEN = "seen "

SERVER = """--- Answers one request with its target.
answer(exchange: internal Exchange) -> Unit
  uses @server: HttpServer[reply]
=
  let target = exchange.request.target
  @server.reply(to: exchange.right, reply: Reply(status: 200, headers: [], body: "seen {target}"))
    if error use unit"""

MAIN = """--- The language form (`spec/31 §1` rule 6): a task per request, until the host stops the run.
pub main() -> Unit
  uses @server: HttpServer[next, reply, close]
  uses @console: Console[print]
  diverges
=
  tasks at most 100 at once
    for ever
      let next = attempt @server.next()
      when next
        is Ok(exchange) ->
          start answer(exchange)
        is Err(failure) -> unit
    on stop
      @server.close()
  @console.print("stopped")"""


def server_files(name, decls=(SERVER,), main=MAIN, imports=("Exchange", "HttpServer", "Reply")):
    """The package of a server: module `srv` with `decls` and `main`, granted the console and the
    one `serve` grant, and its lock."""
    text = join_decls(header(MOD, imports=[("std.host", list(imports))]), *decls, main)
    files = package_files(name, EDITION, {MOD: text}, entry=MOD, grants=["console", GRANT])
    files["ita.lock"] = lock_text()
    return files


def b64(data):
    return base64.b64encode(data).decode("ascii")


def script(connections):
    """The text of a script (`spec/31 §8` rule 6): `connections` are (at, peer, pieces), each
    piece (at, bytes); connection numbers count from 1 in order."""
    lines = []
    for k, (at, peer, pieces) in enumerate(connections, 1):
        lines.append({"at": at, "connection": k, "grant": LISTENER, "peer": peer,
                      "reads": "eager"})
        lines += [{"at": t, "connection": k, "bytes": b64(data)} for t, data in pieces]
    lines.sort(key=lambda line: line["at"])
    return "".join(json.dumps(line, sort_keys=True) + "\n" for line in lines)


def split(rng, data, at):
    """`data` cut at one to four distinct byte boundaries, each piece one instant after the last,
    from `at`."""
    cuts = sorted(rng.sample(range(1, len(data)), min(len(data) - 1, rng.randint(1, 4))))
    pieces, last = [], 0
    for i, cut in enumerate(cuts + [len(data)]):
        pieces.append((at + i, data[last:cut]))
        last = cut
    return pieces


def request(method="GET", target="/a", version="HTTP/1.1", headers=(("Host", "localhost"),),
            body=b"", line_end=b"\r\n", before=b""):
    """The bytes of one request: its line, `headers` as (name, value) pairs of text or bytes, and
    `body`; `line_end` ends the request line, `before` comes ahead of it."""
    def enc(x):
        return x if isinstance(x, bytes) else x.encode("utf-8")
    out = before + enc(method) + b" " + enc(target) + b" " + enc(version) + line_end
    for name, value in headers:
        out += enc(name) + b": " + enc(value) + b"\r\n"
    return out + b"\r\n" + body


def _host(rng):
    return rng.choice(["localhost", "127.0.0.1", "localhost:8080", "127.0.0.1:8080"])


def _path(rng):
    return "/" + rng.choice(["a", "items", "v1/users", "x/y/z", "q"])


def _extra(rng):
    """Zero to two headers that change nothing the case is about."""
    pool = [("Accept", "*/*"), ("User-Agent", "hunt/1"), ("X-Trace", "t1"),
            ("Accept-Language", "pt-BR")]
    return rng.sample(pool, rng.randint(0, 2))


def refused(status):
    return {"status": status}


def answered(target, behind=True, continued=False):
    """A twin: answered `200` with `seen <target>`, the program's target, and the request behind
    it too unless the first closes the connection; `continued` when `100 Continue` comes first."""
    out = {"status": 200, "answers": [SEEN + target] + ([SEEN + "/behind"] if behind else [])}
    if continued:
        out["continue"] = True
    return out


def _heads(rng, *more, host=True):
    """`Host` (unless `host` is false), the case's own headers `more`, and the extra ones, in an
    order the seed picks; `Host` stays first only when the seed says so."""
    hs = list(more) + _extra(rng)
    rng.shuffle(hs)
    if host:
        hs.insert(rng.randint(0, len(hs)), ("Host", _host(rng)))
    return hs


# N1. The request line and line ends (`spec/31 §3` rules 1 and 2).
def _n1(rng, form):
    path = _path(rng)
    method = rng.choice(["GET", "POST"])
    if form == "plain":
        return request(method, path, headers=_heads(rng)), answered(path)
    if form == "http10":
        return (request(method, path, "HTTP/1.0", headers=_heads(rng)),
                answered(path, behind=False))
    if form == "double-space":
        return request(method, " " + path, headers=_heads(rng)), refused(400)
    if form == "leading-empty-line":
        return request(method, path, headers=_heads(rng), before=b"\r\n"), refused(400)
    if form == "tab-separator":
        head = request(method, path, headers=_heads(rng))
        return head.replace(b" ", b"\t", 1), refused(400)
    if form == "trailing-space":
        # A fourth, empty part: not three parts split by single spaces.
        return request(method, path, rng.choice(["HTTP/1.1 ", "HTTP/1.1  "]),
                       headers=_heads(rng)), refused(400)
    if form == "method-not-implemented":
        return (request(rng.choice(["TRACE", "CONNECT", "PROPFIND", "get", "Post"]), path,
                        headers=_heads(rng)), refused(501))
    if form == "method-not-token":
        return request(rng.choice(["GE(T", "G@T", "GET/"]), path, headers=_heads(rng)), refused(400)
    if form == "version":
        # Three parts whose third is not `HTTP/1.1` or `HTTP/1.0`, a tab after it included.
        return (request(method, path, rng.choice(["HTTP/2.0", "HTTP/1.2", "HTTP/0.9", "HTTP/3",
                                                  "HTTP/1.1\t", "http/1.1"]),
                        headers=_heads(rng)), refused(505))
    if form == "target-byte":
        bad = rng.choice(["\x7f", "\x01", "é"])
        return request(method, path + bad + "b", headers=_heads(rng)), refused(400)
    if form == "line-too-long":
        return request(method, "/" + "a" * (LINE_LIMIT + 16), headers=_heads(rng)), refused(414)
    if form == "bare-lf-line":
        return request(method, path, headers=_heads(rng), line_end=b"\n"), refused(400)
    if form == "bare-lf-header":
        head = request(method, path, headers=_heads(rng))
        # The first header line's CRLF becomes a bare LF.
        i = head.index(b"\r\n") + 2
        j = head.index(b"\r\n", i)
        return head[:j] + b"\n" + head[j + 2:], refused(400)
    if form == "bare-cr":
        return (request(method, path, headers=_heads(rng, ("X-Note", b"a\rb"))), refused(400))
    raise ValueError(form)


N1_FORMS = ("plain", "http10", "double-space", "leading-empty-line", "tab-separator",
            "trailing-space",
            "method-not-implemented", "method-not-token", "version", "target-byte",
            "line-too-long", "bare-lf-line", "bare-lf-header", "bare-cr")


# N2. Header lines, encodings and expectations (`spec/31 §3` rules 3 and 6).
def _n2(rng, form):
    path = _path(rng)
    if form == "kept-values":
        return (request("GET", path, headers=_heads(rng, ("X-Spaced", rng.choice(
            ["  v  ", "\tv\t", "a\tb", "a b"])))), answered(path))
    if form == "connection-listed":
        value = rng.choice(["keep-alive", "Keep-Alive", "keep-alive, te", "TE , Keep-Alive"])
        return request("GET", path, headers=_heads(rng, ("Connection", value))), answered(path)
    if form == "identity":
        value = rng.choice(["identity", "IDENTITY", "Identity"])
        return (request("GET", path, headers=_heads(rng, ("Content-Encoding", value))),
                answered(path))
    if form == "continue":
        body = rng.choice([b"x", b"hello", b"{}"])
        hs = _heads(rng, ("Expect", rng.choice(["100-continue", "100-Continue"])),
                    ("Content-Length", str(len(body))))
        return request("POST", path, headers=hs, body=body), answered(path, continued=True)
    if form == "obs-fold":
        head = request("GET", path, headers=_heads(rng, ("X-Folded", "a")))
        i = head.index(b"X-Folded: a\r\n") + len(b"X-Folded: a\r\n")
        return head[:i] + rng.choice([b" b\r\n", b"\tb\r\n"]) + head[i:], refused(400)
    if form == "space-before-colon":
        name = rng.choice(["Host ", "X-A ", "Content-Length\t"])
        hs = [(name, "localhost" if name.startswith("Host") else "0")] + _extra(rng)
        if not name.startswith("Host"):
            hs.append(("Host", _host(rng)))
        return request("GET", path, headers=hs), refused(400)
    if form == "name-not-token":
        name = rng.choice(["X(A)", "X@A", "X/A", "X\x01A", ""])
        return request("GET", path, headers=_heads(rng, (name, "v"))), refused(400)
    if form == "value-control":
        bad = rng.choice([b"a\x00b", b"a\x01b", b"a\x7fb", b"a\x1bb"])
        return request("GET", path, headers=_heads(rng, ("X-Bad", bad))), refused(400)
    if form == "value-not-utf8":
        bad = rng.choice([b"\xff", b"a\xc3", b"\xed\xa0\x80", b"\xc0\xaf"])
        return request("GET", path, headers=_heads(rng, ("X-Bad", bad))), refused(400)
    if form == "connection-other":
        value = rng.choice(["close, x-forwarded-for", "host", "authorization", "foo",
                            "keep-alive, cookie"])
        return request("GET", path, headers=_heads(rng, ("Connection", value))), refused(400)
    if form == "connection-empty-element":
        value = rng.choice(["close,,keep-alive", ",close", "close,", ""])
        return request("GET", path, headers=_heads(rng, ("Connection", value))), refused(400)
    if form == "underscore-name":
        name = rng.choice(["Transfer_Encoding", "Content_Length", "Content_Encoding",
                           "Proxy_Connection", "Keep_Alive", "transfer_encoding"])
        value = {"Transfer_Encoding": "chunked", "transfer_encoding": "chunked",
                 "Content_Length": "0"}.get(name, "x")
        return request("GET", path, headers=_heads(rng, (name, value))), refused(400)
    if form == "encoding":
        value = rng.choice(["gzip", "br", "deflate", "identity, gzip"])
        return (request("GET", path, headers=_heads(rng, ("Content-Encoding", value))),
                refused(415))
    if form == "expectation":
        value = rng.choice(["200-ok", "100-continue, foo", "foo"])
        return request("GET", path, headers=_heads(rng, ("Expect", value))), refused(417)
    if form == "too-many-headers":
        hs = [("Host", "localhost")] + [(f"X-H{i}", "v") for i in range(HEADERS_LIMIT)]
        return request("GET", path, headers=hs), refused(431)
    if form == "header-too-big":
        hs = _heads(rng, ("X-Big", "h" * (HEADER_LIMIT + 8)))
        return request("GET", path, headers=hs), refused(431)
    raise ValueError(form)


N2_FORMS = ("kept-values", "connection-listed", "identity", "continue", "obs-fold",
            "space-before-colon", "name-not-token", "value-control", "value-not-utf8",
            "connection-other", "connection-empty-element", "underscore-name", "encoding",
            "expectation", "too-many-headers", "header-too-big")


def _chunked(chunks, size_text=None, after=b"\r\n", trailers=()):
    """A chunked body: each chunk's size in lower-case hex (or `size_text(i, n)`), its data and
    `after`, the last chunk and the trailers."""
    out = b""
    for i, data in enumerate(chunks):
        size = size_text(i, len(data)) if size_text else format(len(data), "x").encode()
        out += size + b"\r\n" + data + after
    out += b"0\r\n"
    for name, value in trailers:
        out += f"{name}: {value}\r\n".encode()
    return out + b"\r\n"


# N3. Framing, the defense against smuggling (`spec/31 §3` rule 4, and rule 8's UTF-8 body).
def _n3(rng, form):
    path = _path(rng)
    body = rng.choice([b"hello", b"a=1&b=2", b"{\"k\": 1}"])
    cl = ("Content-Length", str(len(body)))
    te = ("Transfer-Encoding", "chunked")
    chunked = _chunked([body])
    if form == "length":
        return request("POST", path, headers=_heads(rng, cl), body=body), answered(path)
    if form == "chunked":
        name = rng.choice(["Transfer-Encoding", "transfer-encoding", "TRANSFER-ENCODING"])
        value = rng.choice(["chunked", "Chunked", "CHUNKED"])
        hex_case = rng.choice([str.lower, str.upper])
        data = _chunked([body, body], size_text=lambda i, n: hex_case(format(n, "x")).encode(),
                        trailers=rng.choice([(), (("X-Sum", "1"),)]))
        return request("POST", path, headers=_heads(rng, (name, value)), body=data), answered(path)
    if form == "length-and-chunked":
        hs = _heads(rng, cl, te)
        return request("POST", path, headers=hs, body=chunked), refused(400)
    if form == "chunked-http10":
        return (request("POST", path, "HTTP/1.0", headers=_heads(rng, te), body=chunked),
                refused(400))
    if form == "two-chunked":
        return request("POST", path, headers=_heads(rng, te, te), body=chunked), refused(400)
    if form == "coding-not-implemented":
        value = rng.choice(["gzip", "gzip, chunked", "deflate", "compress, chunked"])
        return (request("POST", path, headers=_heads(rng, ("Transfer-Encoding", value)),
                        body=chunked), refused(501))
    if form == "coding-malformed":
        value = rng.choice(["chunked, chunked", "", "chunked;q=1", "chunked,", ", chunked",
                            "chunked x"])
        return (request("POST", path, headers=_heads(rng, ("Transfer-Encoding", value)),
                        body=chunked), refused(400))
    if form == "two-lengths":
        other = rng.choice([str(len(body)), str(len(body) + 1)])
        hs = _heads(rng, cl, ("Content-Length", other))
        return request("POST", path, headers=hs, body=body), refused(400)
    if form == "length-malformed":
        value = rng.choice([f"+{len(body)}", f"-{len(body)}", f"{len(body)}, {len(body)}",
                            f"0x{len(body):x}", "0" * 19 + "5", f"{len(body)}.0",
                            f"{len(body)} {len(body)}"])
        hs = _heads(rng, ("Content-Length", value))
        return request("POST", path, headers=hs, body=body), refused(400)
    if form == "length-over-limit":
        value = rng.choice([str(BODY_LIMIT + 1), "9999999999999999999"])
        hs = _heads(rng, ("Content-Length", value))
        return request("POST", path, headers=hs, body=body), refused(413)
    if form == "chunk-size-malformed":
        n = len(body)
        bad = rng.choice([f"{n:x};ext=1", f" {n:x}", f"{n:x} ", "0" * 16 + f"{n:x}",
                          f"+{n:x}", f"0x{n:x}", f"{n:x}\t", ""])
        data = _chunked([body], size_text=lambda i, _n: bad.encode())
        return request("POST", path, headers=_heads(rng, te), body=data), refused(400)
    if form == "chunk-data-unterminated":
        data = _chunked([body], after=rng.choice([b"X\r\n", b"\n", b""]))
        return request("POST", path, headers=_heads(rng, te), body=data), refused(400)
    if form == "chunk-over-limit":
        size = rng.choice([BODY_LIMIT + 1, 2 * BODY_LIMIT])
        data = format(size, "x").encode() + b"\r\n"
        return request("POST", path, headers=_heads(rng, te), body=data), refused(413)
    if form == "body-not-utf8":
        bad = rng.choice([b"\xff", b"a\xc3", b"\xed\xa0\x80"])
        hs = _heads(rng, ("Content-Length", str(len(bad))))
        return request("POST", path, headers=hs, body=bad), refused(400)
    raise ValueError(form)


N3_FORMS = ("length", "chunked", "length-and-chunked", "chunked-http10", "two-chunked",
            "coding-not-implemented", "coding-malformed", "two-lengths", "length-malformed",
            "length-over-limit", "chunk-size-malformed", "chunk-data-unterminated",
            "chunk-over-limit", "body-not-utf8")


# N4. `Host` and the target, path confusion (`spec/31 §3` rule 5, §4).
def _n4(rng, form):
    def one(target, host=None, version="HTTP/1.1"):
        hs = _heads(rng, host=False)
        hs.insert(rng.randint(0, len(hs)), ("Host", host if host is not None else _host(rng)))
        return request("GET", target, version, headers=hs)
    if form == "ordinary-segments":
        target = rng.choice(["/..;/x", "/%252e%252e/x", "/a//b", "/a/..b", "/.a", "/a%20b",
                             "/~user", "/a;b=c"])
        return one(target), answered(target)
    if form == "query":
        target = rng.choice(["/a?x=1&y=2", "/a?v=%0d%0aSet-Cookie:%20x", "/a?&&x", "/a?x=a+b",
                             "/a?q=?/"])
        return one(target), answered(target)
    if form == "absolute":
        host = rng.choice(["localhost:8080", "127.0.0.1:8080"])
        path = rng.choice(["/a", "/a?x=1", ""])
        return one(f"http://{host}{path}", host), answered(path or "/")
    if form == "host-forms":
        host = rng.choice(["LOCALHOST", "localhost.", "Localhost:8080", "[::1]", "[::1]:8080",
                           "127.0.0.1:080"])
        return one("/a", host), answered("/a")
    if form == "host-missing":
        hs = _extra(rng)
        return request("GET", "/a", rng.choice(["HTTP/1.1", "HTTP/1.0"]), headers=hs), refused(400)
    if form == "host-twice":
        hs = _heads(rng, ("Host", _host(rng)))
        return request("GET", "/a", headers=hs), refused(400)
    if form == "host-malformed":
        host = rng.choice(["", "a/b@c", "user@localhost", "127.1", "0x7f.0.0.1", "0x7f",
                           "localhost:", "localhost:65536", "localhost:123456", "local host",
                           "[fe80::1%25en0]", "[::1", "a..b", ".localhost", "localhost:-1"])
        return one("/a", host), refused(400)
    if form == "host-not-granted":
        host = rng.choice(["example.com", "evil.example", "localhost.localdomain", "a.b.c"])
        return one("/a", host), refused(421)
    if form == "target-form":
        target = rng.choice(["*", "localhost:8080", "a/b", "http://evil.example/a",
                             "https://localhost:8080/a", "ftp://localhost/a"])
        return one(target), refused(400)
    if form == "target-chars":
        target = "/a" + rng.choice(["#b", "\\b", "|b", "{b", "\"b", "^b", "`b", "<b", ">b", "[b",
                                    "]b"])
        return one(target), refused(400)
    if form == "dot-segments":
        target = rng.choice(["/./x", "/../x", "/a/..", "/%2e%2e/x", "/%2E%2e/x", "/%2e/x",
                             "/a/%2e%2E"])
        return one(target), refused(400)
    if form == "decoded-segment":
        target = rng.choice(["/a%2fb", "/a%2Fb", "/a%5cb", "/a%00", "/a%0ab", "/a%7f", "/a%1b"])
        return one(target), refused(400)
    if form == "bad-percent":
        target = rng.choice(["/a%zz", "/a%4", "/a%", "/%c3", "/%ff", "/a?x=%ff", "/a?x=%g1",
                             "/a?%"])
        return one(target), refused(400)
    raise ValueError(form)


N4_FORMS = ("ordinary-segments", "query", "absolute", "host-forms", "host-missing",
            "host-twice", "host-malformed", "host-not-granted", "target-form", "target-chars",
            "dot-segments", "decoded-segment", "bad-percent")

RULES = {"N1": "spec/31 §3 rules 1 and 2", "N2": "spec/31 §3 rules 3 and 6",
         "N3": "spec/31 §3 rules 4 and 8", "N4": "spec/31 §3 rule 5, §4"}
CASES = {"N1": (_n1, N1_FORMS), "N2": (_n2, N2_FORMS), "N3": (_n3, N3_FORMS),
         "N4": (_n4, N4_FORMS)}


def connections(rng, data):
    """The script of a request program: `data` (the request and the one behind it) whole on
    connection 1, split on connection 2, and the control request on connection 3, each from a
    peer address of its own and far apart in time."""
    return [(0, "192.0.2.1", [(1, data)]),
            (1000, "192.0.2.2", split(rng, data, 1001)),
            (2000, "192.0.2.3", [(2001, CONTROL)])]


def _request_op(op):
    make, forms = CASES[op]

    def generate(seed, edition):
        if edition not in EDITIONS:
            raise ValueError(f"family N runs edition {EDITION} only, not {edition}")
        rng = rng_for(seed)
        form = rng.choice(forms)
        first, expect = make(rng, form)
        files = server_files("hunt")
        files["script.jsonl"] = script(connections(rng, first + BEHIND))
        kind = "refused" if "answers" not in expect else "answered"
        note = (f"a malformed request: refused {expect['status']}, the request behind it never "
                "read" if kind == "refused" else
                "a well-formed twin: answered 200, and the request behind it")
        return Program(op=op, rule=RULES[op], edition=edition, seed=seed, cls=OPEN, files=files,
                       note=note, variant=form, serve=dict(expect, kind="request"))
    generate.__name__ = op.lower()
    return generate


OPERATORS = {op: _request_op(op) for op in CASES}
