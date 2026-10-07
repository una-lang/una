"""Area 3 of #308: labels across JSON, HTTP, Map and Text. Each case is a package of edition
0.2 with the diagnostic it must give (or none), checked with `una --json check`.
Usage: python3 matrix.py <una binary>"""
import json, os, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
LOCK = os.path.join(HERE, "..", "..", "..", "..", "..", "..", "examples", "wordfreq", "verification.lock")
HEAD = """module main

"""
USES = """  uses @http: Http[get, post]
  uses @env: Env[get]
  uses @console: Console[print]
"""
# name: (expected code or "ok", body of main after the uses clauses)
CASES = {
    "secret-text-into-str": ("U4001", "f(s: secret Text) -> Json = Str(value: s)\n", None),
    "internal-text-into-str": ("U4001", "f(s: internal Text) -> Json = Str(value: s)\n", None),
    "internal-key-into-object": ("U4001", "f(k: internal Text) -> Json = Object(fields: empty_map().with(key: k, value: Null))\n", None),
    "internal-value-map-into-object": ("U4001", "f(m: Map[Text, internal Json]) -> Json = Object(fields: m)\n", None),
    "secret-header-value": ("U4001", "f(s: secret Text) -> Header = Header(name: \"x\", value: s)\n", None),
    "internal-header-value": ("ok", "f(s: internal Text) -> Header = Header(name: \"x\", value: s)\n", None),
    "response-body-parsed-to-console": ("U4003", None, """  let r = @http.get("http://127.0.0.1:8765/", headers: [], key_name: None)
    if error use Response(status: 0, headers: [], body: "null")
  let j = attempt parse_json(r.body)
  when j
    is Ok(v) -> @console.print(v.to_json())
    is Err -> @console.print("bad")
"""),
    "parse-failure-of-internal-to-console": ("U4003", None, """  let t = @env.get("T")
    if none use "null"
  let j = attempt parse_json(t)
  when j
    is Ok -> @console.print("json")
    is Err -> @console.print("not json")
"""),
    "parse-failure-position-to-console": ("U4003", None, """  let t = @env.get("T")
    if none use "null"
  let j = attempt parse_json(t)
  when j
    is Ok -> @console.print("json")
    is Err(e) ->
      when e
        is Syntax(at) -> @console.print("{at}")
        else -> @console.print("other")
"""),
    "parse-if-error-pass-internal": ("U4002", "g(t: internal Text) -> internal Json\n  fails JsonError\n=\n  parse_json(t)\n    if error pass\n", None),
    "quote-json-of-internal-to-console": ("U4003", None, """  let t = @env.get("T")
    if none use ""
  @console.print(t.quote_json())
"""),
    "interpolated-internal-json-to-console": ("U4003", None, """  let t = @env.get("T")
    if none use "1"
  let j = parse_json(t)
    if error use Null
  @console.print("{j}")
"""),
    "map-has-internal-key-to-console": ("U4003", None, """  let k = @env.get("K")
    if none use "a"
  let m: Map[Text, Int] = empty_map().with(key: "a", value: 1)
  if m.has(key: k)
    @console.print("yes")
  else
    @console.print("no")
"""),
    "find-in-internal-to-console": ("U4003", None, """  let t = @env.get("T")
    if none use ""
  when t.find(text: "x")
    is Some -> @console.print("found")
    is None -> @console.print("absent")
"""),
    "status-to-console": ("U4003", None, """  let r = @http.get("http://127.0.0.1:8765/", headers: [], key_name: None)
    if error use Response(status: 0, headers: [], body: "")
  if r.status == 200
    @console.print("ok")
  else
    @console.print("not ok")
"""),
    "secret-url": ("U4003", "h(u: secret Text) -> internal Response\n  uses @http: Http[get]\n  fails HttpError\n=\n  @http.get(u, headers: [], key_name: None)\n    if error pass\n", None),
    "secret-body": ("U4003", "h(b: secret Text) -> internal Response\n  uses @http: Http[post]\n  fails HttpError\n=\n  @http.post(\"http://127.0.0.1:8765/\", headers: [], body: b, key_name: None)\n    if error pass\n", None),
    "secret-key-name": ("U4003", "h(k: secret Text) -> internal Response\n  uses @http: Http[get]\n  fails HttpError\n=\n  @http.get(\"http://127.0.0.1:8765/\", headers: [], key_name: Some(k))\n    if error pass\n", None),
    "internal-body-allowed": ("ok", "h(b: internal Text) -> internal Response\n  uses @http: Http[post]\n  fails HttpError\n=\n  @http.post(\"http://127.0.0.1:8765/\", headers: [], body: b, key_name: None)\n    if error use Response(status: 0, headers: [], body: \"\")\n", None),
    "http-fails-of-internal-url-pass": ("U4002", "h(u: internal Text) -> internal Response\n  uses @http: Http[get]\n  fails HttpError\n=\n  @http.get(u, headers: [], key_name: None)\n    if error pass\n", None),
    "http-fails-of-internal-header-pass": ("U4002", "h(v: internal Text) -> internal Response\n  uses @http: Http[get]\n  fails HttpError\n=\n  @http.get(\"http://127.0.0.1:8765/\", headers: [Header(name: \"x\", value: v)], key_name: None)\n    if error pass\n", None),
    "http-under-internal-pc": ("U4003", None, """  let t = @env.get("T")
    if none use ""
  if t == "go"
    let r = @http.get("http://127.0.0.1:8765/", headers: [], key_name: None)
      if error use Response(status: 0, headers: [], body: "")
    @console.print("done")
  else
    @console.print("skip")
"""),
    "json-of-internal-written-by-http": ("ok", None, """  let t = @env.get("T")
    if none use ""
  let j = parse_json(t)
    if error use Null
  let r = @http.post("http://127.0.0.1:8765/", headers: [], body: j.to_json(), key_name: None)
    if error use Response(status: 0, headers: [], body: "")
  @console.print("sent")
"""),
}


def build(name, fn, main):
    d = os.path.join(HERE, name)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(os.path.join(d, "src"))
    shutil.copy(LOCK, d)
    open(os.path.join(d, "una.toml"), "w").write('[package]\nname = "case"\nversion = "0.1.0"\nedition = "0.2"\n')
    src = HEAD
    if fn:
        src += fn
    if main:
        src += "pub main() -> Unit\n" + USES + "=\n" + main
    open(os.path.join(d, "src", "main.una"), "w").write(src)
    return d


def main():
    una = os.path.abspath(sys.argv[1])
    bad = 0
    for name, (expected, fn, body) in CASES.items():
        d = build(name, fn, body)
        r = subprocess.run([una, "--json", "check", "--tier", "0", d], capture_output=True, text=True)
        doc = json.loads(r.stdout)
        codes = sorted({x["code"] for x in doc["diagnostics"]})
        got = "ok" if not codes else ",".join(codes)
        mark = "  " if got == expected else "XX"
        if got != expected:
            bad += 1
        print(f"{mark} {name}: expected {expected}, got {got}")
        if got != expected and codes:
            for x in doc["diagnostics"]:
                print("      ", x["code"], x["message"][:160])
    print(f"{len(CASES)} cases, {bad} unexpected")


main()
