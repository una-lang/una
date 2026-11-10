"""The two-run oracle (docs/plan/security-hunt-oracle.md), the part the release gate runs.

For a program `una check` accepts, with varied literals (`Program.vary`), each literal is
replaced by other values of its type, one position at a time, and every variant runs on the
interpreter, `una run --json`, twice. What an observer below the literal's label sees
(`project`, at `public` and at `internal`) must be the same in every variant (T6, `spec/07 §1`).
An observer sees an event when it sees its operation's `accepts` (`EvVis`,
formal/UnaFormal/F1/Binary.lean): the host's from the prelude's declarations, the program's own
from its Core export. Scope: V-secret only (source literals behind a label above the observer),
the interpreter. V-host, the Http mock and the final tree of `Files.write` are not part of it.
A fixed sample of the programs, about 5%, also runs natively, every variant, and must agree with
the interpreter on everything a run shows (`native.py`, #792).

It fails closed. Every end other than the ones below is an error, and an error fails the gate:
- `complete`: `ok` (exit 0, no diagnostics) or `failed` (exit 1, a failure of `main`);
- `abort`: `aborted` (exit 1) with only U6003, U6004 or U6005, or the wall-clock limit. A run-time
  abort is termination, outside T6 (`spec/07 §1`; ADR 0061 rule 3). A pair whose one side
  completes and whose other side aborts is a `termination` pair: recorded and counted, never a
  finding, and never an error.
Two runs of one variant that differ are an error (nondeterminism), never averaged away.
A run, or a native build, in which the solver did not decide an obligation (U5002, `unknown:
wallclock` among them) or was missing (U5003) is an error too, never a program set aside: the
runs do the full check with the pinned solver (#813), whose answers are deterministic under its
rlimit (`spec/12 §10`).

**The example probe** (#1911). Since S-41 and S-42 (`spec/11 §5`, `spec/26 §6` rule 4, #1514,
#1515, #1544) a run's trace leaves out every operation performed in quiet code, under a program
counter above `public`, and every operation of a handler instance that quiet code installed: the
trace a run shows is public by construction, so what an observer sees of such operations can no
longer be read from it, in any variant. An example's trace keeps the operations performed in
quiet code (`spec/08 §2` rule 7), though never those of instances quiet code installed. So each
variant also runs once more as an example, the probe (`probe_files`, `run_probe`): the entry
module gets a handler of `Console` that does nothing and a test (`example` in editions 0.1 and
0.2) that gives it as `@console` and runs `main()`, and `una test --json` reports its trace. The
probe's events join every observer's view, projected by `accepts` as the run's are (`EvVis`),
their redacted sinks counted as violations, and the liveness view at the literal's own level,
with their outcomes; the test's own result does not, since the test report of edition 0.2 and
later writes labeled fields of a failure unredacted (ADR 0078). A program whose `main` needs a
host capability other than `Console` has no probe (U3014: an example cannot use host
capabilities), which the result records; the gate pins those reasons and their counts. A probe
whose check the solver did not decide, that aborts or times out while the run completed, whose
report is not a test document, or that is available for the base and not for a variant, is an
error. The cost-clock and server modes run no probe: an example has neither clock nor peers.

**The cost clock** (#973, ADR 0117 S8). With `cost=True`, every run is `una run --virtual-time
--cost-clock` (`spec/27 §8` rule 6): each check point moves the virtual clock by one millisecond, so
a deadline passes, or a token is left at a limit, by how much work the run did. A secret that
decides that work, and a timing label that misses it, then make two variants' values differ, which
the comparison sees. In that mode:
- the trace's events with no operation whose outcome is `timeout` or `cancelled` are not
  compared, though the liveness view at the literal's own level keeps them: whether a `within`
  timed out, and which tasks its cut or a race cancelled, are the timing information the trace
  records by design (`spec/27 §5` rule 7, `spec/26 §7` rule 4);
- a program that names `Clock` is set aside: under the cost clock `Clock.now` reads the run's work,
  timing outside the model (`spec/07 §1`);
- nothing runs natively: executables have no cost clock.

**Servers** (#1011, ADR 0123 S9). With `serve=True`, every run serves the program's script of
connections (`script.jsonl`, `spec/31 §8` rule 6): `una run --virtual-time --serve-script
script.jsonl --serve-out ...` with `UNA_TEST_RUN_SERVER=1`, which only a test build of `una`
serves, and which binds no listener. What the peers received, the lines of `--serve-out`, joins the
run's document as `serve_out`: `reply` accepts `internal` (ADR 0123 IF2), so an observer at
`internal` sees the wire, every byte and its instant, and one at `public` does not. The operations
of `HttpServer` join the table of `accepts` (`SERVER_ACCEPTS`). A program whose policy releases to
`internal` is compared at `public` only: what it shows its peers is what the policy released.
Nothing runs natively: the native scripted engine is a runtime test hook. Without `serve`, nothing
changes: a run document has no `serve_out`, so no view has a wire.
"""

import json
import os
import re
from collections import Counter

from . import native, sandbox
from .program import Program
from .una import module_file

ABORT_CODES = {"U6003", "U6004", "U6005"}
# The solver's own ends in a run: an obligation `unknown` (U5002, `unknown: wallclock` among them)
# or no solver at the pinned version (U5003). The oracle's runs and the native builds do the full
# check, with the pinned solver since #813, so the two-run determinism check rests on its answers
# (rlimit, `spec/12 §10`); either code in a run is a gate error, never a program set aside.
SOLVER_CODES = {"U5002", "U5003"}
# How many values replace one varied literal: a fixed cap, so the gate's size is fixed.
VALUES_PER_POSITION = 5
# The limits of one run (docs/plan/security-hunt-oracle.md §6).
RUN_LIMITS = ["--max-depth", "2000", "--max-memory", "256"]
# The clock of the cost-clock mode (`spec/27 §8` rule 6).
COST_CLOCK = ["--virtual-time", "--cost-clock"]
# The flags of the server mode: the script of connections in the program's own directory, and
# `--serve-out`, whose path the run adds (`spec/31 §8` rule 6).
SERVE_SCRIPT = "script.jsonl"
SERVE_ENV = {"UNA_TEST_RUN_SERVER": "1"}
# The `accepts` of the operations of `HttpServer` (`spec/31 §1` rule 2), in the server mode only.
SERVER_ACCEPTS = {
    ("HttpServer", "next"): "internal",
    ("HttpServer", "reply"): "internal",
    ("HttpServer", "close"): "public",
}
# The outcomes of events with no operation that the cost-clock mode leaves out of every view.
TIMING_OUTCOMES = ("timeout", "cancelled")
# A program that names it is set aside in the cost-clock mode.
CLOCK_WORD = re.compile(r"\bClock\b")
LARGE_INT = 9007199254740993
LONG_TEXT = "x" * 300
NON_ASCII = "ação ☃"
# Text constants of the program the oracle reuses: plain ones only, so that no value needs an
# escape or opens an interpolation.
PLAIN_TEXT = re.compile(r'"([A-Za-z0-9 _.,:-]{0,40})"')
INT_CONST = re.compile(r"(?<![A-Za-z0-9_.])(-?\d+)(?![A-Za-z0-9_.])")


def int_text(n):
    return str(n)


def text_literal(s):
    return '"' + s + '"'


def values(ty, literal, files):
    """The replacement literals for one varied literal of type `ty`, in a fixed order, at most
    VALUES_PER_POSITION, never the original (docs/plan/security-hunt-oracle.md §2)."""
    if ty == "Bool":
        cands = ["false" if literal == "true" else "true"]
    elif ty == "Int":
        consts = sorted({int(m) for text in _sources(files) for m in INT_CONST.findall(text)})
        cands = [int_text(n) for n in [0, -1, LARGE_INT]]
        for c in consts:
            cands += [int_text(c - 1), int_text(c + 1)]
        cands.append(int_text(1))
    elif ty == "Text":
        consts = sorted({m for text in _sources(files) for m in PLAIN_TEXT.findall(text)})
        cands = [text_literal(s) for s in ["", NON_ASCII, LONG_TEXT, "a"]]
        cands += [text_literal(s) for s in consts]
    else:
        raise ValueError(f"no values for the type {ty}")
    out = []
    for c in cands:
        if c != literal and c not in out:
            out.append(c)
    return out[:VALUES_PER_POSITION]


def _sources(files):
    return [text for rel, text in sorted(files.items()) if rel.endswith(".una")]


def substitute(files, pos, text):
    """`files` with the literal at `pos` (file, 1-based line and column) replaced by `text`. The
    literal must be at that place, or the manifest and the program disagree."""
    lines = files[pos["file"]].split("\n")
    line = lines[pos["line"] - 1]
    col = pos["col"] - 1
    if line[col:col + len(pos["literal"])] != pos["literal"]:
        raise ValueError(f"{pos['file']}:{pos['line']}:{pos['col']}: "
                         f"the varied literal {pos['literal']} is not there")
    lines[pos["line"] - 1] = line[:col] + text + line[col + len(pos["literal"]):]
    out = dict(files)
    out[pos["file"]] = "\n".join(lines)
    return out


POLICY_TO = re.compile(r"^\s+to (.+)$", re.M)
LEVELS_ABOVE_PUBLIC = ("internal", "confidential", "secret", "restricted")


def releases_to_public(files):
    """Whether a policy of the program releases to `public`: its `to` names no level above
    `public`. A difference such a program shows is explained by the policy (`spec/07 §1`, §5),
    so the oracle sets the program aside and counts it."""
    for text in _sources(files):
        if not re.search(r"^(pub )?policy ", text, re.M):
            continue
        for to in POLICY_TO.findall(text):
            if not to.strip().startswith(LEVELS_ABOVE_PUBLIC):
                return True
    return False


def releases_to(files, level):
    """Whether a policy of the program releases to `level` exactly: its `to` starts with it."""
    for text in _sources(files):
        if not re.search(r"^(pub )?policy ", text, re.M):
            continue
        if any(to.strip().split("[", 1)[0].split(" ", 1)[0] == level
               for to in POLICY_TO.findall(text)):
            return True
    return False


# Levels, lowest first (spec/07 §2).
LEVELS = ["public", "internal", "confidential", "secret", "restricted"]
# The observers the oracle compares at, each when the varied literal is above it.
OBSERVERS = ("public", "internal")
# The `accepts` of every host operation, as the prelude declares it (crates/una-check/src/std,
# spec/14 §3, spec/24 §5); `Keys.hmac` has no clause, so `public`, the default (spec/07 §7). A unit
# test pins this table to the prelude's declarations.
HOST_ACCEPTS = {
    ("Console", "print"): "public",
    ("Clock", "now"): "public",
    ("Env", "get"): "public",
    ("Files", "read"): "public",
    ("Files", "write"): "internal",
    ("Keys", "hmac"): "public",
    ("Http", "get"): "internal",
    ("Http", "post"): "internal",
    ("Http", "put"): "internal",
    ("Http", "delete"): "internal",
}
# The arguments of an operation that are sinks, where not all are: `Keys.hmac`'s `data` accepts
# every label and is not a sink (spec/14 §3, ADR 0039), so an observer sees only `key_name`.
SINK_ARGS = {("Keys", "hmac"): ("key_name",)}


class UnknownOperation(ValueError):
    """An event of an operation neither the host table nor the program declares: the oracle cannot
    tell who sees it, so it is an error, never dropped."""


def level_of(label):
    """The level of a label written `secret[PII]`, `internal`, or a Core export's label object."""
    if isinstance(label, dict):
        label = label.get("level", "")
    return str(label).split("[", 1)[0].strip()


def sees(observer, accepts_levels):
    """Whether `observer` sees an event whose operation accepts `accepts_levels` (`EvVis`). Fail
    closed: an operation declared more than once is seen when any declaration is, and a level the
    oracle does not know is seen."""
    rank = LEVELS.index(observer)
    for lvl in accepts_levels:
        if lvl not in LEVELS or LEVELS.index(lvl) <= rank:
            return True
    return False


def accepts_table(core_doc):
    """(capability, operation) -> the set of `accepts` levels: the host's, then the program's own
    capabilities from its Core export (`una --json core`), whose `accepts` the compiler resolved.
    An operation with no `accepts` accepts `public` (spec/07 §7)."""
    table = {key: {lvl} for key, lvl in HOST_ACCEPTS.items()}
    for module in core_doc.get("modules", []):
        for decl in module.get("decls", []):
            if decl.get("kind") != "cap":
                continue
            for op in decl.get("ops", []):
                acc = op.get("accepts")
                lvl = level_of(acc) if acc else "public"
                table.setdefault((decl.get("name"), op.get("name")), set()).add(lvl)
    return table


REDACTED = re.compile(r"<redacted:([A-Za-z]+)")
# Fields of a run document that may differ between two runs of one program: none. Two runs whose
# documents differ in anything are nondeterministic, an error.
RUN_VARYING_FIELDS = ()


def above(level, observer):
    """Whether `level` is above `observer`; a level the oracle does not know is (fail closed)."""
    return level not in LEVELS or LEVELS.index(level) > LEVELS.index(observer)


def project(doc, observer, table, keep_outcome=False, cost=False):
    """What an observer at `observer` sees of one run (docs/release-gates.md, the `hunt` gate):
    - `status`, every `output` line in order (`Console.print` accepts `public`), and `failure`
      (its variant and its `public` fields);
    - `diagnostics`, by severity, code and message: `una run --json` redacts what a message
      renders above `public`, so a message that differs between variants shows what reached it
      (#874, the class of S-28);
    - the trace's events whose operation's `accepts` the observer sees (`EvVis`), in order, each
      with every field but `seq` and `outcome`: `seq` numbers every event, the hidden ones too, and
      `outcome` is the host's answer, an input of the run, not something the program sends. For
      `Keys.hmac` only `key_name`, its one sink, is kept of the arguments. An event with no
      operation (a task's cancellation, `spec/19 §7`) is kept whole but for `seq`.
    `violations` lists the visible events whose sink arguments the run redacted at a level above
    the observer: the run itself says the argument is above what the operation accepts, so the
    checks let a labeled value reach the sink, a finding whatever the other variants do.
    `keep_outcome` keeps outcomes too, for the liveness view at the literal's own level. `cost`,
    the cost-clock mode, leaves out the events with no operation whose outcome is in
    `TIMING_OUTCOMES`.
    Raises `UnknownOperation` for an event of an operation `table` does not have."""
    trace, violations = [], []
    for ev in doc.get("trace", []):
        if "capability" not in ev and "op" not in ev:
            if cost and (ev.get("outcome") or {}).get("kind") in TIMING_OUTCOMES:
                continue
            trace.append({k: v for k, v in ev.items() if k != "seq"})
            continue
        key = (ev.get("capability"), ev.get("op"))
        if key not in table:
            raise UnknownOperation(f"an event of {key[0]}.{key[1]}, whose accepts is not known")
        if not sees(observer, table[key]):
            continue
        drop = ("seq",) if keep_outcome else ("seq", "outcome")
        view = {k: v for k, v in ev.items() if k not in drop}
        if key in SINK_ARGS and isinstance(view.get("args"), dict):
            view["args"] = {k: v for k, v in view["args"].items() if k in SINK_ARGS[key]}
        for name, value in sorted((view.get("args") or {}).items()):
            for lvl in REDACTED.findall(str(value)):
                if above(lvl, observer):
                    violations.append(f"{key[0]}.{key[1]}({name}) is <redacted:{lvl}> at a sink "
                                      f"{observer} sees")
        trace.append(view)
    view = {
        "status": doc.get("status"),
        "output": doc.get("output"),
        "failure": doc.get("failure"),
        "diagnostics": [[d.get("severity"), d.get("code"), d.get("message")]
                        for d in doc.get("diagnostics", [])],
        "trace": trace,
        "violations": violations,
    }
    # The server mode: the peers' wire, which `reply` sends at `internal` (ADR 0123 IF2).
    if "serve_out" in doc and not above("internal", observer):
        view["wire"] = doc["serve_out"]
    return view


def observe(run, observers, table, live_level, cost=False, probe=None, probe_observers=None):
    """One run's end: (`complete`, {"views": {observer: view}, "live": view at `live_level`,
    "probe": status}), (`abort`, codes), (`rejected`, codes), or (`error`, why). `probe`, the
    probe's end (`run_probe`), adds its events to each view of `probe_observers` (every observer
    by default) and to the liveness view, as `probe`, or None when the probe is unavailable; its
    status, `available`, `unavailable: <reason>` or None without a probe, is the end's `probe`.
    A probe that errs or aborts is the end's error."""
    if probe is not None and probe[0] == "error":
        return ("error", "probe: " + probe[1])
    end = _observe(run, observers, table, live_level, cost)
    if end[0] != "complete" or probe is None:
        return end
    if probe[0] == "abort":
        return ("error", "probe: the probe aborted while the run completed: " + probe[1])
    views, live = end[1]["views"], end[1]["live"]
    if probe[0] == "unavailable":
        for view in views.values():
            view["probe"] = None
        live["probe"] = None
        end[1]["probe"] = "unavailable: " + probe[1]
        return end
    seen = observers if probe_observers is None else probe_observers
    try:
        for o, view in views.items():
            if o not in seen:
                view["probe"] = None
                continue
            pv = project(probe[1], o, table)
            view["probe"] = pv["trace"]
            view["violations"] = sorted(set(view["violations"]) | set(pv["violations"]))
        live["probe"] = project(probe[1], live_level, table, keep_outcome=True)["trace"]
    except UnknownOperation as e:
        return ("error", "probe: " + str(e))
    end[1]["probe"] = "available"
    return end


def _observe(run, observers, table, live_level, cost=False):
    """`observe` without a probe."""
    if run.timed_out:
        return ("abort", ["timeout"])
    try:
        doc = json.loads(run.stdout)
    except ValueError:
        return ("error", f"exit {run.code}, the output is not a document: "
                         f"{(run.stdout + run.stderr)[-300:]}")
    if not isinstance(doc, dict) or doc.get("schema") != "una.run/1":
        return ("error", f"exit {run.code}, not a una.run/1 document")
    status = doc.get("status")
    diags = doc.get("diagnostics", [])
    codes = sorted({d.get("code") for d in diags if d.get("severity") == "error"})
    solver = sorted({d.get("code") for d in diags} & SOLVER_CODES)
    if solver:
        return ("error", f"status {status!r}, the solver did not decide: {', '.join(solver)}")
    if status == "rejected" and run.code == 1:
        return ("rejected", codes)
    if status == "ok" and run.code == 0 and diags:
        return ("error", "an ok run with diagnostics: " + ", ".join(d.get("code", "?") for d in diags))
    if status == "failed" and any(d.get("severity") != "error" for d in diags):
        return ("error", "a failed run with diagnostics other than errors: "
                         + ", ".join(d.get("code", "?") for d in diags))
    if (status == "ok" and run.code == 0) or (status == "failed" and run.code == 1
                                              and doc.get("failure")):
        try:
            return ("complete", {"views": {o: project(doc, o, table, cost=cost)
                                           for o in observers},
                                 # The liveness view keeps the timing events: that a deadline
                                 # passed for some secrets is what makes the variation live.
                                 "live": project(doc, live_level, table, keep_outcome=True)})
        except UnknownOperation as e:
            return ("error", str(e))
    if status == "aborted" and run.code == 1 and codes and set(codes) <= ABORT_CODES:
        return ("abort", codes)
    return ("error", f"status {status!r}, exit {run.code}, codes {codes}")


def judge(ref, other):
    """The verdict on a variant's end `other` against `ref`, the program's first complete end
    (the base's, or the first variant's that completed), or None when no end has completed yet:
    - `same` or `leak`: two complete ends, compared at every observer; a difference at any is a
      leak, whatever the base did;
    - `termination`: one side completes and the other aborts (outside T6, `spec/07 §1`);
    - `abort`: no end has completed yet and this one aborts too;
    - `error`: an error on either side."""
    if other[0] == "error" or (ref is not None and ref[0] == "error"):
        return "error"
    if other[0] == "complete":
        if ref is None:
            return "first"
        return "same" if ref[1]["views"] == other[1]["views"] else "leak"
    if other[0] == "abort":
        return "abort" if ref is None else "termination"
    return "error"


def leaking_observers(ref, other):
    """The observers at which two complete ends differ."""
    if ref[0] != "complete" or other[0] != "complete":
        return []
    a, b = ref[1]["views"], other[1]["views"]
    return sorted(o for o in a if a[o] != b.get(o))


def violations(end):
    """The redacted sinks a complete end shows its observers."""
    if end[0] != "complete":
        return []
    return sorted({v for view in end[1]["views"].values() for v in view.get("violations", [])})


def literal_level(vary):
    """The lowest level among a program's varied literals; a level the oracle does not know counts
    as `restricted`."""
    return min((level_of(p["label"]) if level_of(p["label"]) in LEVELS else "restricted"
                for p in vary), key=LEVELS.index)


def observers_for(vary):
    """The observers a program's varied literals are above: every one of OBSERVERS strictly below
    the lowest level among them."""
    lowest = LEVELS.index(literal_level(vary))
    return [o for o in OBSERVERS if LEVELS.index(o) < lowest]


def export_core(una, pkg, root, timeout):
    """The program's Core export (`una --json core`), complete, or an error string."""
    r = sandbox.run_fresh([una, "--json", "core", "."], cwd=pkg, root=root, timeout=timeout)
    if r.timed_out or r.code != 0:
        return None, "the Core export " + ("timed out" if r.timed_out else f"exited {r.code}")
    try:
        doc = json.loads(r.stdout)
    except ValueError:
        return None, "the Core export is not JSON"
    if doc.get("complete") is not True:
        return None, "the Core export is not complete"
    return doc, None


# What `una core` says of a program with dependencies (`spec/30 §12` rule 7).
NO_CORE_2 = "una.core/2"


def dependency_export(una, pkg, root, timeout):
    """None when `una core` refuses the program of several packages at `pkg` as a usage error
    naming `una.core/2`, as this release does; otherwise why, an error of the gate: an export
    that starts to be written must be read by the oracle and the re-check, never skipped."""
    r = sandbox.run_fresh([una, "--json", "core", "."], cwd=pkg, root=root, timeout=timeout)
    if r.timed_out:
        return "the Core export of a program with dependencies timed out"
    if r.code == 2 and NO_CORE_2 in r.stderr + r.stdout:
        return None
    return (f"the Core export of a program with dependencies exited {r.code}, not the usage "
            f"error that names {NO_CORE_2}: the oracle must read it")


def checks_accept(una, pkg, root, timeout):
    """Whether `una check --tier 0` accepts the package at `pkg`: exit 0 and no error."""
    r = sandbox.run_fresh([una, "--json", "check", "--tier", "0", "."], cwd=pkg, root=root,
                          timeout=timeout)
    if r.timed_out or r.code != 0:
        return False
    try:
        doc = json.loads(r.stdout)
    except ValueError:
        return False
    return not any(d.get("severity") == "error" for d in doc.get("diagnostics", []))


def raw(run):
    """What two runs of one variant must share: every byte of the run document, its exit code and
    whether it timed out, minus RUN_VARYING_FIELDS (none)."""
    try:
        doc = json.loads(run.stdout)
        if isinstance(doc, dict):
            doc = {k: v for k, v in doc.items() if k not in RUN_VARYING_FIELDS}
    except ValueError:
        doc = run.stdout
    return (run.timed_out, run.code, doc)


# The example probe (the module's docstring, #1911): the handler of `Console` that does nothing
# and the test that gives it and runs `main()`, appended to the entry module. The test's name and
# the handler's are the probe's own, so they collide with nothing a generator writes.
PROBE_HANDLER = "hunt_probe_console"
PROBE_TEST = "hunt probe"
# The keyword of the test declaration by edition (`spec/01`, ADR 0105): `example` in editions 0.1
# and 0.2, `test` from 0.3 on, where `example` is no longer a declaration.
PROBE_EXAMPLE_EDITIONS = ("0.1", "0.2")
# The schemas of `una test --json` and the key under which each lists the tests (ADR 0105 rule 6).
PROBE_SCHEMAS = {"una.test/1": "examples", "una.test/2": "tests"}
TOML_EDITION = re.compile(r'^edition\s*=\s*"([^"]*)"', re.M)
TOML_ENTRY = re.compile(r'^entry\s*=\s*"([^"]*)"', re.M)
# The same in `ita.una`, the manifest of edition 0.4 packages (`spec/30 §2`): `edition X` in the
# `ita` block, `entry X` in the `run` section.
ITA_EDITION = re.compile(r"^  edition\s+(\S+)", re.M)
ITA_ENTRY = re.compile(r"^run\n(?:  (?!entry\b)[^\n]*\n)*  entry\s+(\S+)", re.M)


def manifest_of(files):
    """The package's edition and entry module, from `una.toml` or `ita.una`: (edition, entry),
    either None when the manifest does not say; or None when there is no manifest."""
    text = files.get("una.toml")
    if isinstance(text, str):
        edition, entry = TOML_EDITION.search(text), TOML_ENTRY.search(text)
    else:
        text = files.get("ita.una")
        if not isinstance(text, str):
            return None
        edition, entry = ITA_EDITION.search(text), ITA_ENTRY.search(text)
    return (edition.group(1) if edition else None, entry.group(1) if entry else None)


def probe_files(files):
    """The variant's files with the probe appended to its entry module, and None; or None and
    the reason there is none: no manifest (`una.toml` or `ita.una`), no entry in it, or no
    module file for the entry."""
    found = manifest_of(files)
    if found is None:
        return None, "no manifest"
    edition, entry = found
    if entry is None:
        return None, "no entry"
    path = module_file(entry)
    module = files.get(path)
    if not isinstance(module, str):
        return None, f"no module file for the entry {entry}"
    keyword = "example" if edition in PROBE_EXAMPLE_EDITIONS else "test"
    decls = (f"handler {PROBE_HANDLER}() for Console\n  op print(line) = unit\n\n"
             f'{keyword} "{PROBE_TEST}"\n  given @console = {PROBE_HANDLER}()\n'
             "  do main()\n  expect true\n")
    return dict(files, **{path: module.rstrip("\n") + "\n\n" + decls}), None


def run_probe(una, files, name, root, timeout):
    """One run of the probe of a variant, in a fresh directory of its own: (`doc`, a document
    whose `trace` is the test's), (`unavailable`, why), (`abort`, why) or (`error`, why). The
    test's own result is not in the document (the module's docstring). Unavailable: no probe for
    the files (`probe_files`), or a check that rejects the probed package, by its codes, among
    them U3014, a `main` that needs a host capability other than `Console`."""
    probed, why = probe_files(files)
    if probed is None:
        return ("unavailable", why)
    rundir = sandbox.fresh_dir(root, f"{name}-probe-")
    pkg = Program(op="oracle", rule="", edition="", seed=0, cls="open", files=probed,
                  variant=name + "-probe").write(rundir)
    r = sandbox.run([una, "--json", "test", ".", *RUN_LIMITS, *native.SOLVER_FLAGS], cwd=pkg,
                    writable=rundir, timeout=timeout, env_extra=native.RUN_ENV)
    if r.timed_out:
        return ("abort", "timeout")
    try:
        doc = json.loads(r.stdout)
    except ValueError:
        return ("error", f"exit {r.code}, the probe's output is not a document: "
                         f"{(r.stdout + r.stderr)[-300:]}")
    if not isinstance(doc, dict) or doc.get("schema") not in PROBE_SCHEMAS:
        return ("error", f"exit {r.code}, not a test document")
    diags = doc.get("diagnostics", [])
    solver = sorted({d.get("code") for d in diags} & SOLVER_CODES)
    if solver:
        return ("error", "the solver did not decide: " + ", ".join(solver))
    codes = sorted({d.get("code") for d in diags if d.get("severity") == "error"})
    if codes:
        return ("unavailable", ", ".join(codes))
    if diags:
        return ("error", "diagnostics other than errors: "
                         + ", ".join(d.get("code", "?") for d in diags))
    tests = [t for t in doc.get(PROBE_SCHEMAS[doc["schema"]], []) if t.get("name") == PROBE_TEST]
    if len(tests) != 1 or r.code not in (0, 1):
        return ("error", f"exit {r.code}, {len(tests)} probe tests in the document")
    result = tests[0].get("result") or {}
    if result.get("kind") == "aborted":
        return ("abort", tests[0].get("reason") or "aborted")
    if result.get("kind") not in ("ok", "fail"):
        return ("error", f"the probe's result is {result.get('kind')!r}")
    return ("doc", {"trace": tests[0].get("trace", [])})


def run_twice(una, files, name, root, timeout, observers, table, live_level, cost=False,
              serve=False, probe=False, probe_observers=None):
    """Runs the variant twice, each run in a fresh directory of its own (its package, home and
    temporary files), and returns its end; an error when the two runs' documents differ. `cost`
    runs both under the cost clock, `serve` over the program's script of connections; `probe`
    also runs the example probe once (`run_probe`), whose events join the views of
    `probe_observers` (`observe`)."""
    return run_twice_kept(una, files, name, root, timeout, observers, table, live_level, None,
                          cost, serve, probe, probe_observers)[0]


def with_serve_out(run, out):
    """`run` with the lines of `--serve-out`, the file at `out`, in its document as `serve_out`:
    a list of the lines' objects, or the text of a file that is not JSON lines, or None when the
    run wrote none. A run whose output is not a document is left as it is."""
    try:
        doc = json.loads(run.stdout)
    except ValueError:
        return run
    if not isinstance(doc, dict):
        return run
    wire = None
    if os.path.isfile(out) and not os.path.islink(out):
        with open(out, encoding="utf-8", errors="replace") as f:
            text = f.read()
        try:
            wire = [json.loads(line) for line in text.splitlines()]
        except ValueError:
            wire = text
    doc["serve_out"] = wire
    return sandbox.Run(run.code, json.dumps(doc), run.stderr, run.timed_out, run.seconds)


def run_twice_kept(una, files, name, root, timeout, observers, table, live_level,
                   doc_plant=None, cost=False, serve=False, probe=False, probe_observers=None):
    """`run_twice`, and the first of the two runs, for the native comparison. `doc_plant`, for the
    gate's canary only, rewrites the run's document, given the variant's files. `cost` runs both
    under the cost clock (`COST_CLOCK`); `serve` over the script of connections, with what the
    peers received in the document (`with_serve_out`). `probe` runs the example probe once after
    the two runs, when they completed (`run_probe`, `observe`)."""
    clock = COST_CLOCK if cost else []
    runs = []
    for k in range(2):
        rundir = sandbox.fresh_dir(root, f"{name}-r{k}-")
        p = Program(op="oracle", rule="", edition="", seed=0, cls="open", files=files,
                    variant=name)
        pkg = p.write(rundir)
        if serve:
            out = os.path.join(rundir, "serve-out.jsonl")
            r = sandbox.run([una, "--json", "run", ".", *RUN_LIMITS, *clock, "--virtual-time",
                             "--serve-script", SERVE_SCRIPT, "--serve-out", out,
                             *native.SOLVER_FLAGS],
                            cwd=pkg, writable=rundir, timeout=timeout,
                            env_extra={**native.RUN_ENV, **SERVE_ENV})
            runs.append(with_serve_out(r, out))
            continue
        runs.append(sandbox.run([una, "--json", "run", ".", *RUN_LIMITS, *clock,
                                 *native.SOLVER_FLAGS],
                                cwd=pkg, writable=rundir, timeout=timeout,
                                env_extra=native.RUN_ENV))
    if raw(runs[0]) != raw(runs[1]):
        return ("error", "nondeterministic: two runs of one variant differ"), runs[0]
    if runs[0].timed_out:
        # A run killed at its limit is an abort only when its full check decides (#813).
        why = native.undecided(una, files, name, root)
        if why:
            return ("error", why), runs[0]
    run = runs[0]
    if doc_plant is not None and not run.timed_out:
        run = sandbox.Run(run.code, json.dumps(doc_plant(json.loads(run.stdout), files)),
                          run.stderr, run.timed_out, run.seconds)
    end = _observe(run, observers, table, live_level, cost)
    if not probe or end[0] != "complete":
        return end, runs[0]
    probed = run_probe(una, files, name, root, timeout)
    return observe(run, observers, table, live_level, cost, probed, probe_observers), runs[0]


def native_check(result, una, files, name, root, timeout, end, run, native_files=None):
    """Runs the variant natively when its program is in the sample, and records the comparison in
    `result["native"]`: a backend divergence makes the result a finding, a build that fails other
    than "not compiled" an error. `native_files`, for the gate's canary only, builds other files
    than the ones the interpreter ran."""
    record = result["native"]
    if not record["sampled"] or record["not_compiled"] or result["error"] or run is None:
        return
    if end[0] not in ("complete", "abort"):
        return
    if run.timed_out:
        record["skipped"].append({"variant": name, "why": "the interpreter's run timed out"})
        return
    try:
        doc = json.loads(run.stdout)
    except ValueError:
        result["error"] = f"native {name}: the interpreter's run is not a document"
        return
    kind, got = native.build_and_run(una, native_files or files, name, root, timeout, RUN_LIMITS)
    if kind == "not compiled":
        record["not_compiled"] = got
        return
    if kind == "build failed":
        result["error"] = f"native {name}: una build failed: {got}"
        return
    nrun, ntrace = got
    differences = native.compare(run, doc, nrun, ntrace)
    record["compared"] += 1
    if differences:
        record["divergences"].append({"variant": name, "differences": differences})
        result["finding"] = True


def native_pass(prog, una, root, timeout, result):
    """The native comparison of a sampled program outside the oracle's own runs: the stratum of
    its family (`native.py`), which the oracle has run, or a program the oracle set aside because
    a policy releases to public. Each variant, the base first, runs once on the interpreter and
    natively, and the two are compared as `native_check` does. Updates `result["native"]`."""
    result["native"].update(sampled=True, stratum=True)
    variants = [(prog.name + "-base", prog.files)]
    for i, pos in enumerate(prog.vary):
        for j, text in enumerate(values(pos["type"], pos["literal"], prog.files)):
            variants.append((f"{prog.name}-v{i}-{j}", substitute(prog.files, pos, text)))
    for name, files in variants:
        rundir = sandbox.fresh_dir(root, f"{name}-i-")
        p = Program(op="oracle", rule="", edition="", seed=0, cls="open", files=files,
                    variant=name)
        pkg = p.write(rundir)
        run = sandbox.run([una, "--json", "run", ".", *RUN_LIMITS, *native.SOLVER_FLAGS], cwd=pkg,
                          writable=rundir, timeout=timeout, env_extra=native.RUN_ENV)
        if run.timed_out:
            # As in the oracle's runs: an abort only when the full check decides; `native_check`
            # then lists the variant as not compared.
            why = native.undecided(una, files, name, root)
            if why:
                result["error"] = result["error"] or f"native {why}"
                return
            # Counted here, once: this run is the native pass's own, not one of the oracle's.
            result["native"]["pass_timed_out"] = result["native"].get("pass_timed_out", 0) + 1
            native_check(result, una, files, name, root, timeout, ("abort", None), run)
            continue
        try:
            doc = json.loads(run.stdout)
            status = doc.get("status")
            codes = {d.get("code") for d in doc.get("diagnostics", [])}
        except (ValueError, AttributeError):
            doc, status, codes = None, None, set()
        if codes & SOLVER_CODES:
            # The run's full check did not decide: a gate error, never a run left out (#813).
            undecided_codes = ", ".join(sorted(codes & SOLVER_CODES))
            result["error"] = result["error"] or (
                f"native {name}: the solver did not decide: {undecided_codes}")
            return
        if status in ("ok", "failed"):
            end = ("complete", None)
        elif status == "aborted":
            end = ("abort", None)
        else:
            result["error"] = result["error"] or (
                f"native {name}: the interpreter's run ends {status!r}, exit {run.code}"
                if doc is not None else f"native {name}: the interpreter's run is not a document")
            return
        native_check(result, una, files, name, root, timeout, end, run)


def oracle(rows, una, root, timeout, out, rebuild, plant=None, native_plant=None, doc_plant=None,
           cost=False, serve=False):
    """Runs the oracle on every manifest row with varied literals. `rebuild` turns a row into its
    program (`drivers.rebuild`); `plant`, for the gate's canaries only, rewrites the table of
    `accepts`; `native_plant`, for the native canary only, puts the program in the native sample and
    gives the files its executables are built from; `doc_plant`, for the message canary only,
    rewrites each run's document. Writes one result per program to `out` and returns the tally.

    A program's result counts `compared`, its complete-against-complete comparisons, and says
    whether it is `live`: whether two of its complete ends differ at the literal's own level, where
    every event below it is visible with its outcome.

    `cost` is the cost-clock mode (the module's docstring): every run under the cost clock, a
    program that names `Clock` set aside, and no native sample. `serve` is the server mode: every
    run over the program's script of connections, the wire in the `internal` view, the operations
    of `HttpServer` in the table, a program whose policy releases to `internal` compared at
    `public` only, and no native sample. In neither mode does the example probe run; otherwise
    each variant runs it once, and the result's `probe` says whether it was available."""
    tally = Counter()
    probe = not cost and not serve
    for row in rows:
        if not row.get("vary"):
            continue
        prog = rebuild(row)
        observers = observers_for(prog.vary)
        if serve and releases_to(prog.files, "internal"):
            # What the peers see is what the policy released: only `public` stays to compare.
            observers = [o for o in observers if o == "public"]
        # The probe's events join the `internal` view unless a policy releases to `internal`:
        # then what that observer sees of them is what the policy released.
        probe_observers = [o for o in observers
                           if o == "public" or not releases_to(prog.files, "internal")]
        live_level = literal_level(prog.vary)
        result = {"name": prog.name, "op": prog.op, "seed": prog.seed, "edition": prog.edition,
                  "class": prog.cls, "observers": observers, "variants": [], "finding": False,
                  "error": None, "termination": [], "violations": [], "compared": 0,
                  "live": False, "probe": None,
                  "native": {"sampled": not cost and not serve
                                        and (native_plant is not None
                                             or native.sampled(prog.name)),
                             "stratum": False, "compared": 0, "divergences": [], "skipped": [],
                             "not_compiled": None}}
        if not observers:
            result["set_aside"] = "no observer below the varied literal"
        elif releases_to_public(prog.files):
            result["set_aside"] = "a policy releases to public"
        elif cost and any(isinstance(t, str) and CLOCK_WORD.search(t)
                          for t in prog.files.values()):
            result["set_aside"] = "it reads the clock, which the cost clock moves with its work"
        if result.get("set_aside"):
            # A program whose policy releases to public still runs natively when sampled: a
            # declassification is where redaction could differ between the engines.
            if result["native"]["sampled"] and releases_to_public(prog.files) and observers:
                native_pass(prog, una, root, timeout, result)
                result["native"]["stratum"] = False
            tally["set aside"] += 1
            out.write(json.dumps(result, sort_keys=True) + "\n")
            continue
        pkg = Program(op="oracle", rule="", edition="", seed=0, cls="open", files=prog.files,
                      variant=prog.name + "-core").write(root)
        if prog.has_dependencies:
            # A program of several packages has no Core export in this release (`una.core/2`,
            # `spec/30 §12` rule 7): the export must refuse it so. A program the checks accept
            # is judged by the host's `accepts` alone, so an event of any other operation is an
            # error (`UnknownOperation`); one they reject goes the way of an incomplete export.
            why = dependency_export(una, pkg, root, timeout)
            if why is not None:
                result["error"] = why
                tally["program error"] += 1
                out.write(json.dumps(result, sort_keys=True) + "\n")
                continue
            accepted = checks_accept(una, pkg, root, timeout)
            core, why = ({"modules": []}, None) if accepted else (None, "the checks reject it")
        else:
            core, why = export_core(una, pkg, root, timeout)
        if core is None:
            # A program the checks reject has no complete export: the base run says so.
            base = run_twice(una, prog.files, prog.name + "-base", root, timeout, observers,
                             {k: {v} for k, v in HOST_ACCEPTS.items()}, live_level, cost,
                             serve)
            if base[0] == "rejected" and not set(base[1]) & set(prog.expect):
                # Only a closed call or a must-reject program is set aside as rejected, and only
                # with one of its expected codes: another rejection tests nothing of its rule.
                result["error"] = ("rejected by " + (", ".join(base[1]) or "no code") + ", not "
                                   + (", ".join(prog.expect) or "an expected code"))
                tally["program error"] += 1
            elif base[0] == "rejected":
                result["set_aside"] = "rejected by the checks: " + ", ".join(base[1])
                tally["set aside (rejected)"] += 1
            else:
                result["error"] = "base: " + why
                tally["program error"] += 1
            out.write(json.dumps(result, sort_keys=True) + "\n")
            continue
        table = accepts_table(core)
        if serve:
            for key, lvl in SERVER_ACCEPTS.items():
                table.setdefault(key, set()).add(lvl)
        if plant is not None:
            table = plant(table)
        base, base_run = run_twice_kept(una, prog.files, prog.name + "-base", root, timeout,
                                        observers, table, live_level, doc_plant, cost, serve,
                                        probe, probe_observers)
        if base[0] == "rejected":
            result["error"] = "the checks reject a program whose Core export is complete"
        elif base[0] == "error":
            result["error"] = "base: " + base[1]
        elif base[0] == "complete":
            result["probe"] = base[1].get("probe")
        native_check(result, una, prog.files, prog.name + "-base", root, timeout, base, base_run,
                     native_plant(prog.files) if native_plant else None)
        result["base_timed_out"] = base == ("abort", ["timeout"])
        ref = base if base[0] == "complete" else None
        completes = [base] if base[0] == "complete" else []
        for i, pos in enumerate(prog.vary):
            if result["error"]:
                break
            for j, text in enumerate(values(pos["type"], pos["literal"], prog.files)):
                files = substitute(prog.files, pos, text)
                end, end_run = run_twice_kept(una, files, f"{prog.name}-v{i}-{j}", root, timeout,
                                              observers, table, live_level, doc_plant, cost,
                                              serve, probe, probe_observers)
                native_check(result, una, files, f"{prog.name}-v{i}-{j}", root, timeout, end,
                             end_run, native_plant(files) if native_plant else None)
                if end[0] == "rejected":
                    end = ("error", "a variant is rejected by the checks: " + ", ".join(end[1]))
                elif end[0] == "complete" and ref is not None and ref[0] == "complete" \
                        and end[1].get("probe") != ref[1].get("probe"):
                    # The probe is a view of the program, so its availability cannot depend on
                    # the literal; a variant whose probe differs is an error, never a comparison.
                    end = ("error", f"the probe is {end[1].get('probe')} for a variant and "
                                    f"{ref[1].get('probe')} for the reference")
                if end[0] == "complete" and result["probe"] is None:
                    result["probe"] = end[1].get("probe")
                verdict = judge(ref, end)
                entry = {"position": i, "value": text, "verdict": verdict}
                if end == ("abort", ["timeout"]):
                    # Killed at its limit, its full check deciding (`native.undecided`): counted,
                    # so a drift in such kills shows before it reaches a floor (#818).
                    entry["timed_out"] = True
                if verdict == "first":
                    # The first complete end: the reference of the rest. Against an aborted
                    # base, the pair is a termination pair.
                    ref = end
                    if base[0] == "abort":
                        verdict = entry["verdict"] = "termination"
                if verdict in ("same", "leak"):
                    result["compared"] += 1
                if verdict == "leak":
                    entry["observers"] = leaking_observers(ref, end)
                    result["finding"] = True
                if end[0] == "complete":
                    completes.append(end)
                result["variants"].append(entry)
                tally["variant " + verdict] += 1
                if verdict == "termination":
                    other = base if end[0] == "complete" else ref
                    result["termination"].append(
                        {"value": text,
                         "base": other[0] if other[0] == "complete" else other[1],
                         "variant": end[0] if end[0] == "complete" else end[1]})
                elif verdict == "error":
                    result["error"] = end[1] if end[0] == "error" else f"ends {ref[0] if ref else None} and {end[0]}"
                    break
        found = sorted({v for e in completes for v in violations(e)})
        if found:
            result["violations"] = found
            result["finding"] = True
        lives = [e[1]["live"] for e in completes]
        result["live"] = any(x != lives[0] for x in lives[1:])
        if result["native"]["sampled"]:
            tally["native program"] += 1
            tally["native variants compared"] += result["native"]["compared"]
            tally["native divergence"] += len(result["native"]["divergences"])
        tally["program " + ("error" if result["error"] else
                            "finding" if result["finding"] else "compared")] += 1
        out.write(json.dumps(result, sort_keys=True) + "\n")
        out.flush()
    return tally
