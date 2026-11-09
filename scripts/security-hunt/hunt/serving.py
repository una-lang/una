"""The hunt's server run (#1011, ADR 0123 S9): family N through scripts of connections.

    python3 -B -m hunt servers --una PATH --out DIR [--seed 1011] [--count 40] [--z3 PATH]

Each program of family N (`hunt/families/servers.py`) is a server and a script of connections
(`spec/31 §8` rule 6). It runs twice, `una run --virtual-time --serve-script ... --serve-out ...`,
each run in a fresh directory of its own under the hunt's sandbox, and what its peers received
(`--serve-out`) is judged against the program's expectation (`Program.serve`). The runs bind no
listener: the scripted engine takes the script's connections in place of listeners, and the
sandbox denies every network operation.

The scripted engine serves only in the toolchain's own test builds (the `test-hooks` feature of
`una-cli`, as `cargo test` builds `target/debug/una`), with `UNA_TEST_RUN_SERVER=1`; any other
`una` refuses the script with a usage error, and the run stops before any program with that
reason, never a program set aside.

It is not the release gate's fixed run (`gate.py`, seed 767), and it never changes it: family N is
in `SERVER_OPERATORS`, apart from the operators the gate runs, and has a seed of its own. It runs
at release validation or overnight, as heavy work (`docs/release-gates.md`, "The hunt's server
run"). It fails closed: `summary.json` says PASS only when there is no finding and no error, the
canaries behave, and every form of every operator ran; a run of fewer than `COUNT` seeds per
operator, a smoke run, is `INCOMPLETE` at best, never a PASS.

A finding: what a peer received differs from the expectation (a refusal answered otherwise, a
request behind a refusal read, a twin refused, a reply with an injected header sent or its
injection on the wire, an echoed header not sent once as given), a split request read otherwise
than whole, a reply the program sent that a peer cannot frame, a malformed request that ended the
server, or the engine left not at rest at the run's end. An error: a run that does not end `ok` with `stopped`,
two runs of one program that differ, or a `--serve-out` that is not the format of `spec/31 §8`.

The canaries:
- `canary-answered`: a well-formed request, judged by its own expectation, must be clean;
- `canary-planted`: the same run judged against a refusal it did not get must be a finding, so a
  judge that passes everything fails the run.
"""

import atexit
import base64
import binascii
import json
import os
import random
import re
import time
from collections import Counter

from . import drivers, native, oracle, sandbox
from .families import SERVER_OPERATORS
from .families import servers
from .gate import _sharded
from .program import OPEN, Program, derive_seed, write_manifest

SEED = 1011
# Programs per operator, before the seeds that reach its remaining forms (`programs`).
COUNT = 40
# How far past the first `COUNT` seeds `programs` looks for a seed of a form not reached.
SEARCH = 1000
EDITION = servers.EDITION
RUN_TIMEOUT = 30
WORKERS = 4
SCRIPT = "script.jsonl"
# The usage error of a `una` whose scripted engine does not serve (`spec/31 §8` rule 6).
PENDING = "do not run in this release yet"
# The statuses that carry no body (RFC 9110 §15.2, §15.3.5, §15.4.5).
NO_BODY = (100, 204, 304)


# The `Date` header the host writes on every reply but `100` (`spec/31 §5` rule 2), an IMF-fixdate
# of the engine's clock: the one part of the bytes two connections at other instants may differ in.
DATE = re.compile(rb"\r\nDate: (Mon|Tue|Wed|Thu|Fri|Sat|Sun), \d\d "
                  rb"(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{4} "
                  rb"\d\d:\d\d:\d\d GMT\r\n")


def undated(data):
    """`data` with each `Date` header's value left out."""
    return DATE.sub(b"\r\nDate: -\r\n", data)


class WireError(ValueError):
    """Bytes a peer received that are not a sequence of HTTP/1.1 replies."""


def programs(seed=SEED, count=COUNT):
    """Every program of the server run, in a fixed order: for each operator of
    `SERVER_OPERATORS`, edition 0.4, its first `count` seeds, then, for each of its forms those
    did not reach, the first seed past them, within `SEARCH`, whose program has it, so that every
    form runs whatever the count. Raises when a form has no such seed."""
    out = []
    for op in sorted(SERVER_OPERATORS):
        forms = servers.FORMS[op]
        progs = [SERVER_OPERATORS[op](derive_seed(seed, op, i), EDITION) for i in range(count)]
        missing = set(forms) - {p.variant for p in progs}
        for i in range(count, count + SEARCH):
            if not missing:
                break
            p = SERVER_OPERATORS[op](derive_seed(seed, op, i), EDITION)
            if p.variant in missing:
                progs.append(p)
                missing.discard(p.variant)
        if missing:
            raise ValueError(f"the server run of {op} has no seed for the forms "
                             f"{', '.join(sorted(missing))}")
        out += progs
    return out


def read_wire(text):
    """`--serve-out`'s text (`spec/31 §8` rule 6): {connection: {"bytes": the bytes received, in
    order, "ends": the set of `shutdown`, `closed` and `refused`}}. Raises `WireError` on a line
    that is not one of its forms."""
    wire = {}
    for n, line in enumerate(text.splitlines(), 1):
        try:
            row = json.loads(line)
        except ValueError:
            raise WireError(f"line {n} of --serve-out is not JSON") from None
        if not isinstance(row, dict) or not isinstance(row.get("connection"), int):
            raise WireError(f"line {n} of --serve-out names no connection")
        conn = wire.setdefault(row["connection"], {"bytes": b"", "ends": set()})
        kinds = set(row) - {"at", "connection"}
        if kinds == {"bytes"}:
            try:
                conn["bytes"] += base64.b64decode(row["bytes"], validate=True)
            except (binascii.Error, TypeError):
                raise WireError(f"line {n} of --serve-out has bytes that are not base64") from None
        elif kinds in ({"shutdown"}, {"closed"}, {"refused"}):
            conn["ends"] |= kinds
        else:
            raise WireError(f"line {n} of --serve-out is not a form of spec/31 §8 rule 6")
    return wire


def replies(data):
    """The HTTP/1.1 replies in `data`, in order, each (status, [(name in lower case, value)],
    body). Raises `WireError` when the bytes are not a whole sequence of replies, each framed by
    its `Content-Length` (every reply of `spec/31 §5` rule 2 but `100`, `204` and `304` has one).
    It reads replies to `GET` and `POST` only: a reply to `HEAD` keeps a length with no body."""
    out, i = [], 0
    while i < len(data):
        end = data.find(b"\r\n\r\n", i)
        if end < 0:
            raise WireError(f"a reply at byte {i} has no end of its head")
        lines = data[i:end].split(b"\r\n")
        parts = lines[0].split(b" ", 2)
        if len(parts) < 2 or parts[0] != b"HTTP/1.1" or not parts[1].isdigit() \
                or len(parts[1]) != 3:
            raise WireError(f"a reply at byte {i} has the status line {lines[0][:80]!r}")
        status = int(parts[1])
        headers = []
        for raw in lines[1:]:
            name, sep, value = raw.partition(b":")
            if not sep or not name or name != name.strip() or b"\r" in raw or b"\n" in raw:
                raise WireError(f"a reply at byte {i} has the header line {raw[:80]!r}")
            try:
                headers.append((name.decode("ascii").lower(), value.strip().decode("utf-8")))
            except UnicodeDecodeError:
                raise WireError(f"a reply at byte {i} has a header that is not text") from None
        i = end + 4
        lengths = [v for k, v in headers if k == "content-length"]
        if status in NO_BODY:
            if lengths and status != 304:
                raise WireError(f"a {status} reply at byte {end} carries a content-length")
            body = b""
        else:
            if len(lengths) != 1 or not lengths[0].isdigit():
                raise WireError(f"a {status} reply at byte {end} is framed by {lengths!r}, not one "
                                "content-length")
            n = int(lengths[0])
            if i + n > len(data):
                raise WireError(f"a {status} reply at byte {end} ends before its body")
            body, i = data[i:i + n], i + n
        out.append((status, headers, body))
    return out


def _header(headers, name):
    return [v for k, v in headers if k == name]


def judge_request(expect, wire, reply_events):
    """The findings of a request program (family N1 to N4) on its wire and the number of `reply`
    events of its trace; empty when it is clean."""
    findings = []
    if sorted(wire) != [1, 2, 3]:
        return [f"the peers' connections are {sorted(wire)}, not 1, 2 and 3"]
    try:
        got = replies(wire[1]["bytes"])
    except WireError as e:
        return [f"connection 1: {e}"]
    if "answers" not in expect:
        want = expect["status"]
        if [r[0] for r in got] != [want]:
            findings.append(f"connection 1: a request refused {want} got the replies "
                            f"{[r[0] for r in got]}")
        else:
            status, headers, body = got[0]
            if _header(headers, "content-length") != ["0"] or body:
                findings.append(f"connection 1: the refusal {want} carries a body")
            if _header(headers, "connection") != ["close"]:
                findings.append(f"connection 1: the refusal {want} has no Connection: close")
        if not wire[1]["ends"] & {"shutdown", "closed"}:
            findings.append("connection 1: the connection stays open after a refusal")
    else:
        want = ([100] if expect.get("continue") else []) + [200] * len(expect["answers"])
        if [r[0] for r in got] != want:
            findings.append(f"connection 1: a well-formed request expected {want}, got "
                            f"{[r[0] for r in got]}")
        else:
            bodies = [r[2].decode("utf-8", "replace") for r in got if r[0] == 200]
            if bodies != expect["answers"]:
                findings.append(f"connection 1: the program answered {bodies}, not "
                                f"{expect['answers']}")
            if _header(got[-1][1], "connection") != ["close"]:
                findings.append("connection 1: its last reply has no Connection: close")
        if "closed" not in wire[1]["ends"]:
            findings.append("connection 1: the connection is not closed after its last reply")
    if any(SEEN_BEHIND in r[2] for r in got) and "answers" not in expect:
        findings.append("connection 1: the request behind a refusal was read and answered")
    if undated(wire[2]["bytes"]) != undated(wire[1]["bytes"]):
        findings.append("connection 2: the same bytes, split, received another reply than whole "
                        "(spec/31 §3 rule 10)")
    if bool(wire[2]["ends"] & {"shutdown", "closed"}) != bool(wire[1]["ends"] &
                                                             {"shutdown", "closed"}):
        findings.append("connection 2: the same bytes, split, end the connection otherwise")
    try:
        control = replies(wire[3]["bytes"])
    except WireError as e:
        control = []
        findings.append(f"connection 3: {e}")
    if [(r[0], r[2]) for r in control] != [(200, b"seen /control")]:
        findings.append("connection 3: a valid request after the malformed one is not answered "
                        f"200 (got {[r[0] for r in control]})")
    answered = 2 * len(expect.get("answers", [])) + 1
    if reply_events != answered:
        findings.append(f"the program replied {reply_events} times, not {answered}: it saw a "
                        "request the host should have refused, or missed one it should not")
    return findings


SEEN_BEHIND = (servers.SEEN + "/behind").encode()


def judge_reply(expect, wire, _reply_events):
    """The findings of a reply program (N5) on its wire: a
    header the host must refuse makes `reply` fail with `BadReply` and send nothing, the right
    kept (`spec/31 §5` rule 1), so the peer receives the handler's `502` naming the failure, or
    the host's `500` when the handler leaves the right; a header it must send goes as given,
    once. Nothing of an injection, `expect["marker"]`, may reach the wire. The `reply` events are
    not counted: the handler's fallback replies under a program counter its failure raises, quiet
    code whose operations append no event (`spec/11 §5`, `spec/26 §6` rule 4)."""
    if sorted(wire) != [1]:
        return [f"the peers' connections are {sorted(wire)}, not 1"]
    data = wire[1]["bytes"]
    findings = []
    if expect["marker"].encode() in data.lower():
        findings.append("an injected header or body reached the wire (response splitting)")
    try:
        got = replies(data)
    except WireError as e:
        return findings + [f"connection 1: {e}"]
    if expect["status"] == "refused":
        want, body = (502, b"bad reply") if expect["mode"] == "fallback" else (500, b"")
        if [(r[0], r[2]) for r in got] != [(want, body)]:
            findings.append(f"connection 1: a reply the host must refuse ended in "
                            f"{[(r[0], r[2][:40]) for r in got]}, not [({want}, {body!r})]")
    else:
        name, value = expect["header"]
        if [(r[0], r[2]) for r in got] != [(200, b"ok")]:
            findings.append(f"connection 1: a reply with an echoed header ended in "
                            f"{[(r[0], r[2][:40]) for r in got]}, not [(200, b'ok')]")
        line = f"\r\n{name}: {value}\r\n".encode()
        if data.count(line) != 1:
            findings.append(f"connection 1: the echoed header {name!r} is not sent once as given")
    if "closed" not in wire[1]["ends"]:
        findings.append("connection 1: the connection is not closed after its reply")
    return findings


JUDGES = {"request": judge_request, "reply": judge_reply}


def judge(prog, wire, reply_events):
    """The findings of `prog` on its wire, by the judge its expectation names."""
    kind = prog.serve.get("kind")
    if kind not in JUDGES:
        raise ValueError(f"{prog.name}: no judge for the expectation {kind!r}")
    return JUDGES[kind](prog.serve, wire, reply_events)


def run_once(una, prog, root, timeout):
    """One run of `prog` in a fresh directory: (the run, the text of `--serve-out` or None)."""
    rundir = sandbox.fresh_dir(root, f"{prog.name}-")
    pkg = prog.write(rundir)
    out = os.path.join(rundir, "serve-out.jsonl")
    run = sandbox.run([una, "--json", "run", ".", *oracle.RUN_LIMITS, "--virtual-time",
                       "--serve-script", SCRIPT, "--serve-out", out, *native.SOLVER_FLAGS],
                      cwd=pkg, writable=rundir, timeout=timeout,
                      env_extra={**native.RUN_ENV, "UNA_TEST_RUN_SERVER": "1"})
    text = None
    if os.path.isfile(out) and not os.path.islink(out):
        with open(out, encoding="utf-8") as f:
            text = f.read()
    return run, text


class NotATestBuild(SystemExit):
    """The `una` given refuses scripts of connections: not a test build."""


def end_of(run, text):
    """(`ok`, document) for a run that ended `ok` with `stopped` and wrote `--serve-out`, else
    (`error`, why). A `una` that is not a test build raises `NotATestBuild`."""
    if run.code == 2 and PENDING in run.stderr:
        raise NotATestBuild("hunt servers: this una does not serve scripts of connections; build "
                            "it with `cargo build -p una-cli --features test-hooks` (spec/31 §8 "
                            "rule 6)")
    if run.timed_out:
        return "error", "the run reached its time limit"
    try:
        doc = json.loads(run.stdout)
    except ValueError:
        return "error", f"exit {run.code}, the output is not a document: {run.stderr[-300:]}"
    if run.code != 0 or doc.get("status") != "ok" or doc.get("diagnostics") \
            or doc.get("output") != ["stopped"]:
        return "error", (f"exit {run.code}, status {doc.get('status')!r}, output "
                         f"{doc.get('output')!r}, codes "
                         f"{[d.get('code') for d in doc.get('diagnostics', [])]}: "
                         f"{run.stderr[-300:]}")
    if text is None:
        return "error", "the run wrote no --serve-out"
    return "ok", doc


def serve_program(prog, una, root, timeout=RUN_TIMEOUT):
    """The result row of one program: run twice, the two compared, the second judged."""
    row = {"name": prog.name, "op": prog.op, "variant": prog.variant, "finding": False,
           "findings": []}
    first = run_once(una, prog, root, timeout)
    second = run_once(una, prog, root, timeout)
    if (first[0].code, first[0].stdout, first[1]) != (second[0].code, second[0].stdout,
                                                       second[1]):
        row["error"] = "nondeterministic: two runs of one program differ"
        return row
    kind, doc = end_of(*first)
    if kind == "error":
        row["error"] = doc
        return row
    try:
        wire = read_wire(first[1])
    except WireError as e:
        row["error"] = str(e)
        return row
    replies_sent = sum(1 for ev in doc.get("trace", [])
                       if (ev.get("capability"), ev.get("op")) == ("HttpServer", "reply"))
    findings = judge(prog, wire, replies_sent)
    rest = [line for line in first[0].stderr.splitlines() if line.startswith("warning:")]
    if rest:
        findings.append("the engine was left not at rest: " + "; ".join(rest)[:300])
    row["findings"] = findings
    row["finding"] = bool(findings)
    return row


def serve_rows(rows, una, root, out, timeout=RUN_TIMEOUT):
    """`serve_program` over manifest rows, each rebuilt from its seed; one JSON line per row."""
    tally = Counter()
    for r in rows:
        prog = drivers.rebuild(r)
        res = serve_program(prog, una, root, timeout)
        tally["error" if res.get("error") else "finding" if res["finding"] else "clean"] += 1
        out.write(json.dumps(res, sort_keys=True) + "\n")
    return tally


def canary_program():
    """The canaries' program: a well-formed request and the request behind it."""
    first = servers.request("GET", "/canary", headers=[("Host", "localhost")])
    files = servers.server_files("canary")
    rng = random.Random(SEED)
    files[SCRIPT] = servers.script(servers.connections(rng, first + servers.BEHIND))
    return Program(op="canary-answered", rule="the server run's own canary", edition=EDITION,
                   seed=0, cls=OPEN, files=files,
                   serve={"kind": "request", **servers.answered("/canary")})


def canaries(una, root):
    """The canaries' rows and the errors they give: `canary-answered` must be clean, and the same
    wire judged against a refusal (`canary-planted`) must be a finding."""
    prog = canary_program()
    row = serve_program(prog, una, root)
    errors = []
    if row.get("error") or row["finding"]:
        errors.append(f"canary-answered: a well-formed request is not clean ({row.get('error')!r}, "
                      f"{row['findings']})")
    planted = Program(**{**prog.__dict__, "op": "canary-planted",
                         "serve": {"kind": "request", **servers.refused(400)}})
    prow = serve_program(planted, una, root)
    if prow.get("error") or not prow["finding"]:
        errors.append("canary-planted: a refusal the host did not give is not seen "
                      f"({prow.get('error')!r})")
    return [row, prow], errors


def coverage_errors(progs, rows):
    """A form of an operator that no row ran is an error: the run did not reach it."""
    forms = {(op, f) for op, fs in servers.FORMS.items() for f in fs}
    ran = {(r["op"], r["variant"]) for r in rows if not r.get("error")}
    return [f"servers: the form {f} of {op} did not run" for op, f in sorted(forms - ran)
            if any(p.op == op for p in progs)]


def run(una, out_dir, seed=SEED, count=COUNT, z3=None):
    """Runs the server run. Writes OUT/manifest.jsonl, serve.jsonl, canaries.jsonl and
    summary.json, prints the summary, and returns 0 (PASS) or 1 (FAIL)."""
    if os.path.exists(out_dir) and os.listdir(out_dir):
        raise SystemExit(f"hunt servers: {out_dir} is not empty; the run writes a fresh directory")
    started = time.monotonic()
    sandbox.require()
    sandbox.READABLE[:] = [os.path.dirname(os.path.realpath(p)) for p in (una, z3) if p]
    sandbox.OUT_ROOT[:] = [os.path.realpath(out_dir)]
    key_home = sandbox.make_proof_key()
    atexit.register(sandbox.drop_proof_key, key_home)
    root = drivers.campaign_root(out_dir)
    native.RUN_ENV.clear()
    if z3:
        native.RUN_ENV.update(drivers.solver_env(z3))
    work = os.path.join(root, "work")
    os.makedirs(work, exist_ok=True)
    # The canaries first: a `una` that is not a test build stops the run here.
    canary_rows, errors = canaries(una, os.path.join(work, "canaries"))
    progs = programs(seed, count)
    if len({p.name for p in progs}) != len(progs):
        raise SystemExit("two programs of the server run share a name")
    write_manifest(os.path.join(root, "manifest.jsonl"), progs)
    rows = drivers.read_manifest(os.path.join(root, "manifest.jsonl"))

    def shard(k):
        path = os.path.join(work, f"part{k}")
        os.makedirs(path, exist_ok=True)
        return path
    tally, serve_out = _sharded(lambda part, buf, k: serve_rows(part, una, shard(k), buf), rows,
                                WORKERS)
    findings = []
    for r in serve_out:
        if r.get("error"):
            errors.append(f"serve {r['name']}: {r['error'][:300]}")
        for f in r.get("findings", []):
            findings.append(f"serve {r['name']}: {f}")
    errors.extend(coverage_errors(progs, serve_out))
    for name, lines in (("serve.jsonl", serve_out), ("canaries.jsonl", canary_rows)):
        with open(os.path.join(root, name), "w", encoding="utf-8") as f:
            for line in lines:
                f.write(json.dumps(line, sort_keys=True) + "\n")
    # A run below `COUNT` seeds per operator, a smoke run, is never a PASS.
    verdict = "FAIL" if errors or findings else "PASS" if count >= COUNT else "INCOMPLETE"
    summary = {
        "verdict": verdict,
        "seed": seed,
        "count": count,
        "programs": len(progs),
        "operators": dict(sorted(Counter(p.op for p in progs).items())),
        "serve": dict(sorted(tally.items())),
        "findings": findings,
        "errors": errors,
        "seconds": round(time.monotonic() - started, 1),
    }
    if count < COUNT:
        summary["partial"] = f"{count} seeds per operator, below {COUNT}: a smoke run"
    with open(os.path.join(root, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2, sort_keys=True)
        f.write("\n")
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if verdict == "PASS" else 1
