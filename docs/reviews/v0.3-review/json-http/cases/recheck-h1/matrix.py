"""The recheck of H-1 (#310, PR #317, N-39 and N-40) for #308: each argument that decides an
operation's failure, with a control that must stay accepted, in every method, a loop,
`together`, a generic helper and `fails of`; Files and Keys in both editions; and programs
that must not be over-labeled. Usage: python3 matrix.py <una binary>"""
import json, os, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
LOCK = os.path.join(HERE, "..", "..", "..", "..", "..", "..", "examples", "wordfreq", "verification.lock")
U = "http://127.0.0.1:8765/"
GET = "  uses @http: Http[get, post, put, delete]\n"
CASES = {}


def case(name, expected, src, edition="0.2"):
    CASES[name] = (edition, expected, src)


def fn(params, body, extra="", ret="internal Response"):
    return f"f({params}) -> {ret}\n{GET}{extra}  fails HttpError\n=\n{body}"


# 1. The URL decides: every method, against a public control.
for m, args in [("get", 'headers: [], key_name: None'), ("post", 'headers: [], body: "b", key_name: None'),
                ("put", 'headers: [], body: "b", key_name: None'), ("delete", 'headers: [], key_name: None')]:
    case(f"url-internal-{m}", "U4002", fn("u: internal Text", f"  @http.{m}(u, {args})\n    if error pass\n"))
    case(f"url-public-{m}", "ok", fn("u: Text", f"  @http.{m}(u, {args})\n    if error pass\n"))
    case(f"url-internal-{m}-if-error-fail", "U4002", fn("u: internal Text", f"  @http.{m}(u, {args})\n    if error fail Unreachable\n"))
    case(f"url-internal-{m}-if-error-use", "ok", fn("u: internal Text", f"  @http.{m}(u, {args})\n    if error use Response(status: 0, headers: [], body: \"\")\n"))

# 2. The key name decides.
case("key-internal", "U4002", fn("k: internal Text", f'  @http.get("{U}", headers: [], key_name: Some(k))\n    if error pass\n'))
case("key-public", "ok", fn("k: Text", f'  @http.get("{U}", headers: [], key_name: Some(k))\n    if error pass\n'))

# 3. The headers decide through the list's labels and the names, not the values (N-40).
case("headers-list-internal", "U4002", fn("t: internal Text", f'  let hs = if t == "x" then [Header(name: "a", value: "b")] else []\n  @http.get("{U}", headers: hs, key_name: None)\n    if error pass\n'))
case("headers-value-internal", "ok", fn("v: internal Text", f'  @http.get("{U}", headers: [Header(name: "a", value: v)], key_name: None)\n    if error pass\n'))
case("headers-public", "ok", fn("", f'  @http.get("{U}", headers: [Header(name: "a", value: "b")], key_name: None)\n    if error pass\n'))

# 4. The body does not decide.
case("body-internal-post", "ok", fn("b: internal Text", f'  @http.post("{U}", headers: [], body: b, key_name: None)\n    if error pass\n'))
case("body-internal-put", "ok", fn("b: internal Text", f'  @http.put("{U}", headers: [], body: b, key_name: None)\n    if error pass\n'))

# 5. The branch on the failure, with `attempt`, reaches a public sink only from public arguments.
MAIN = "pub main() -> Unit\n  uses @http: Http[get]\n  uses @env: Env[get]\n  uses @console: Console[print]\n=\n"
case("attempt-internal-url-branch", "U4003", MAIN + f'  let u = @env.get("U")\n    if none use "{U}"\n  let r = attempt @http.get(u, headers: [], key_name: None)\n  when r\n    is Ok -> @console.print("y")\n    is Err -> @console.print("n")\n')
case("attempt-public-url-branch", "ok", MAIN + f'  let r = attempt @http.get("{U}", headers: [], key_name: None)\n  when r\n    is Ok -> @console.print("y")\n    is Err -> @console.print("n")\n')
case("attempt-internal-value-branch", "ok", MAIN + f'  let v = @env.get("V")\n    if none use "x"\n  let r = attempt @http.get("{U}", headers: [Header(name: "a", value: v)], key_name: None)\n  when r\n    is Ok -> @console.print("y")\n    is Err -> @console.print("n")\n')
case("attempt-internal-body-branch", "ok", "pub main() -> Unit\n  uses @http: Http[post]\n  uses @env: Env[get]\n  uses @console: Console[print]\n=\n" + f'  let b = @env.get("B")\n    if none use "x"\n  let r = attempt @http.post("{U}", headers: [], body: b, key_name: None)\n  when r\n    is Ok -> @console.print("y")\n    is Err -> @console.print("n")\n')
case("attempt-internal-error-payload", "U4003", MAIN + f'  let u = @env.get("U")\n    if none use "{U}"\n  let r = attempt @http.get(u, headers: [], key_name: None)\n  when r\n    is Ok -> @console.print("y")\n    is Err(e) -> @console.print("{{e}}")\n')

# 6. A loop, together, a generic helper, `fails of`.
case("loop-internal-urls", "U4002", fn("us: List[internal Text]", f'  let edit last: internal Response = Response(status: 0, headers: [], body: "")\n  for u in us\n    last = @http.get(u, headers: [], key_name: None)\n      if error pass\n  last\n'))
case("loop-public-urls", "ok", fn("us: List[Text]", f'  let edit last: internal Response = Response(status: 0, headers: [], body: "")\n  for u in us\n    last = @http.get(u, headers: [], key_name: None)\n      if error pass\n  last\n'))
case("together-internal-url", "U4003", MAIN + f'  let u = @env.get("U")\n    if none use "{U}"\n  together\n    let a = attempt @http.get(u, headers: [], key_name: None)\n    let b = attempt @http.get("{U}", headers: [], key_name: None)\n  when a\n    is Ok -> @console.print("y")\n    is Err -> @console.print("n")\n')
case("together-public-url", "ok", MAIN + f'  together\n    let a = attempt @http.get("{U}", headers: [], key_name: None)\n    let b = attempt @http.get("{U}", headers: [], key_name: None)\n  when a\n    is Ok -> @console.print("y")\n    is Err -> @console.print("n")\n')
case("generic-helper-internal", "U4002", "pick[T](a: T, b: T) -> T = a\n\n" + fn("u: internal Text", f'  let url = pick(u, b: "{U}")\n  @http.get(url, headers: [], key_name: None)\n    if error pass\n'))
case("generic-helper-public", "ok", "pick[T](a: T, b: T) -> T = a\n\n" + fn("u: Text", f'  let url = pick(u, b: "{U}")\n  @http.get(url, headers: [], key_name: None)\n    if error pass\n'))
# A request that fails is passed on through `fails of`: the inner function's `if error pass` is
# where an internal URL is caught; a lambda cannot hold a handling clause on its line.
RELAY = "relay(us: List[internal Text], each: fn(internal Text) -> internal Response) -> List[internal Response]\n  uses of each\n  fails of each\n=\n  us.map(each)\n    if error pass\n\n"
FETCH_I = "fetch(x: internal Text) -> internal Response\n  uses @http: Http[get]\n  fails HttpError\n=\n  @http.get(x, headers: [], key_name: None)\n    if error pass\n\n"
FETCH_P = "fetch(x: internal Text) -> internal Response\n  uses @http: Http[get]\n  fails HttpError\n=\n  @http.get(\"%s\", headers: [Header(name: \"q\", value: x)], key_name: None)\n    if error pass\n\n" % U
case("fails-of-relay-internal", "U4002", RELAY + FETCH_I + MAIN + f'  let u = @env.get("U")\n    if none use "{U}"\n  let r = attempt relay([u], each: fetch)\n  when r\n    is Ok -> @console.print("y")\n    is Err -> @console.print("n")\n')
case("fails-of-relay-public", "ok", RELAY + FETCH_P + MAIN + f'  let u = @env.get("U")\n    if none use "{U}"\n  let r = attempt relay([u], each: fetch)\n  when r\n    is Ok -> @console.print("y")\n    is Err -> @console.print("n")\n')

# 7. Files and Keys, in both editions.
for ed in ("0.1", "0.2"):
    W = "  uses @files: Files[read, write]\n  fails FileError\n"
    case(f"files-write-internal-path-{ed}", "U4002", f"g(p: internal Text) -> Unit\n{W}=\n  @files.write(p, content: \"x\")\n    if error pass\n", ed)
    case(f"files-write-internal-content-{ed}", "ok", f"g(c: internal Text) -> Unit\n{W}=\n  @files.write(\"out.txt\", content: c)\n    if error pass\n", ed)
    case(f"files-write-public-{ed}", "ok", f"g() -> Unit\n{W}=\n  @files.write(\"out.txt\", content: \"x\")\n    if error pass\n", ed)
    case(f"files-read-internal-path-{ed}", "U4003", f"g(p: internal Text) -> internal Text\n{W}=\n  @files.read(p)\n    if error pass\n", ed)
    K = "  uses @keys: Keys[hmac]\n  fails KeyError\n"
    case(f"keys-secret-data-{ed}", "ok", f"g(d: secret Text) -> secret Hashed\n{K}=\n  @keys.hmac(\"k\", data: d)\n    if error pass\n", ed)
    case(f"keys-internal-name-{ed}", "U4003", f"g(n: internal Text) -> internal Hashed\n{K}=\n  @keys.hmac(n, data: \"d\")\n    if error pass\n", ed)


def main():
    una = os.path.abspath(sys.argv[1])
    bad = 0
    for name, (edition, expected, body) in CASES.items():
        d = os.path.join(HERE, name)
        shutil.rmtree(d, ignore_errors=True)
        os.makedirs(os.path.join(d, "src"))
        shutil.copy(LOCK, d)
        open(os.path.join(d, "una.toml"), "w").write(f'[package]\nname = "case"\nversion = "0.1.0"\nedition = "{edition}"\n')
        open(os.path.join(d, "src", "main.una"), "w").write("module main\n\n" + body)
        subprocess.run([una, "fmt", d], capture_output=True)
        r = subprocess.run([una, "--json", "check", "--tier", "0", d], capture_output=True, text=True)
        diags = json.loads(r.stdout)["diagnostics"]
        got = ",".join(sorted({x["code"] for x in diags})) or "ok"
        mark = "  " if got == expected else "XX"
        bad += got != expected
        print(f"{mark} {name}: expected {expected}, got {got}")
        if got != expected:
            for x in diags:
                print("      ", x["code"], x["primary"]["start"], x["message"][:170])
    print(f"{len(CASES)} cases, {bad} unexpected")


main()
