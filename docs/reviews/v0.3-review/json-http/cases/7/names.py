"""Area 7 of #308: the names of spec/23 §1 rule 2 and spec/24 §1 rule 2 in both editions.
Usage: python3 names.py <una binary>"""
import json, os, shutil, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__))
LOCK = os.path.join(HERE, "..", "..", "..", "..", "..", "..", "examples", "wordfreq", "verification.lock")
# import-std-host-http-0.2: U1001 on the candidate 5495b61, which predates #307; ok from 79ba46a.
CASES = {
    # name: (edition, expected, source)
    "json-kind-0.1": ("0.1", "ok", "pub kind Json = Null | Str(value: Text)\n\npub kind Http = Get | Post\n\npub kind Response\n  status: Int\n\npub f() -> Json = Str(value: \"a\")\n"),
    "parse-json-0.1": ("0.1", "ok", "pub parse_json(t: Text) -> Text = t\n\npub to_json(t: Text) -> Text = t\n"),
    "prelude-json-unreachable-0.1": ("0.1", "U1001", "pub f() -> Text = \"1\".to_json()\n"),
    "http-unreachable-0.1": ("0.1", "U1001", "pub f() -> Int\n  uses @http: Http[get]\n=\n  1\n"),
    "json-kind-0.2": ("0.2", "U1002", "pub kind Json = Null | Other\n"),
    "json-error-kind-0.2": ("0.2", "U1002", "pub kind JsonError = Bad | Worse\n"),
    "json-number-kind-0.2": ("0.2", "U1002", "pub kind JsonNumber = Int\n"),
    "http-kind-0.2": ("0.2", "U1002", "pub kind Response\n  status: Int\n"),
    "http-capability-0.2": ("0.2", "U1002", "pub capability Http\n  op get(url: Text) -> Text\n"),
    "header-kind-0.2": ("0.2", "U1002", "pub kind Header = H | I\n"),
    "http-error-kind-0.2": ("0.2", "U1002", "pub kind HttpError = E | F\n"),
    "variant-shadows-0.2": ("0.2", "ok", "pub kind Mine = Null | Str(value: Int)\n\npub f() -> Mine = Str(value: 1)\n"),
    "import-std-host-http-0.2": ("0.2", "U1001", "import std.host (Http, Response)\n\npub f(r: Response) -> Int = r.status\n"),
}
una = os.path.abspath(sys.argv[1])
bad = 0
for name, (edition, expected, body) in CASES.items():
    d = os.path.join(HERE, name)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(os.path.join(d, "src"))
    shutil.copy(LOCK, d)
    open(os.path.join(d, "una.toml"), "w").write(f'[package]\nname = "case"\nversion = "0.1.0"\nedition = "{edition}"\n')
    open(os.path.join(d, "src", "main.una"), "w").write("module main\n\n" + body)
    r = subprocess.run([una, "--json", "check", "--tier", "0", d], capture_output=True, text=True)
    codes = sorted({x["code"] for x in json.loads(r.stdout)["diagnostics"]})
    got = ",".join(codes) or "ok"
    if got != expected:
        bad += 1
        print(f"XX {name}: expected {expected}, got {got}")
        for x in json.loads(r.stdout)["diagnostics"]:
            print("     ", x["code"], x["message"][:150])
    else:
        print(f"   {name}: {got}")
print(f"{len(CASES)} cases, {bad} unexpected")
