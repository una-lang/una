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
"""

import json
import re
from collections import Counter

from . import native, sandbox
from .program import Program

ABORT_CODES = {"U6003", "U6004", "U6005"}
# How many values replace one varied literal: a fixed cap, so the gate's size is fixed.
VALUES_PER_POSITION = 5
# The limits of one run (docs/plan/security-hunt-oracle.md §6).
RUN_LIMITS = ["--max-depth", "2000", "--max-memory", "256"]
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


def project(doc, observer, table, keep_outcome=False):
    """What an observer at `observer` sees of one run (docs/release-gates.md, the `hunt` gate):
    - `status`, every `output` line in order (`Console.print` accepts `public`), and `failure`
      (its variant and its `public` fields);
    - `diagnostics`, by severity and code;
    - the trace's events whose operation's `accepts` the observer sees (`EvVis`), in order, each
      with every field but `seq` and `outcome`: `seq` numbers every event, the hidden ones too, and
      `outcome` is the host's answer, an input of the run, not something the program sends. For
      `Keys.hmac` only `key_name`, its one sink, is kept of the arguments. An event with no
      operation (a task's cancellation, `spec/19 §7`) is kept whole but for `seq`.
    `violations` lists the visible events whose sink arguments the run redacted at a level above
    the observer: the run itself says the argument is above what the operation accepts, so the
    checks let a labeled value reach the sink, a finding whatever the other variants do.
    `keep_outcome` keeps outcomes too, for the liveness view at the literal's own level.
    Raises `UnknownOperation` for an event of an operation `table` does not have."""
    trace, violations = [], []
    for ev in doc.get("trace", []):
        if "capability" not in ev and "op" not in ev:
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
    return {
        "status": doc.get("status"),
        "output": doc.get("output"),
        "failure": doc.get("failure"),
        "diagnostics": [[d.get("severity"), d.get("code")] for d in doc.get("diagnostics", [])],
        "trace": trace,
        "violations": violations,
    }


def observe(run, observers, table, live_level):
    """One run's end: (`complete`, {"views": {observer: view}, "live": view at `live_level`}),
    (`abort`, codes), (`rejected`, codes), or (`error`, why)."""
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
            return ("complete", {"views": {o: project(doc, o, table) for o in observers},
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
    r = sandbox.run([una, "--json", "core", "."], cwd=pkg, writable=root, timeout=timeout)
    if r.timed_out or r.code != 0:
        return None, "the Core export " + ("timed out" if r.timed_out else f"exited {r.code}")
    try:
        doc = json.loads(r.stdout)
    except ValueError:
        return None, "the Core export is not JSON"
    if doc.get("complete") is not True:
        return None, "the Core export is not complete"
    return doc, None


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


def run_twice(una, files, name, root, timeout, observers, table, live_level):
    """Runs the variant twice, each run in a fresh directory of its own (its package, home and
    temporary files), and returns its end; an error when the two runs' documents differ."""
    return run_twice_kept(una, files, name, root, timeout, observers, table, live_level)[0]


def run_twice_kept(una, files, name, root, timeout, observers, table, live_level):
    """`run_twice`, and the first of the two runs, for the native comparison."""
    runs = []
    for k in range(2):
        rundir = sandbox.fresh_dir(root, f"{name}-r{k}-")
        p = Program(op="oracle", rule="", edition="", seed=0, cls="open", files=files,
                    variant=name)
        pkg = p.write(rundir)
        runs.append(sandbox.run([una, "--json", "run", ".", *RUN_LIMITS], cwd=pkg,
                                writable=rundir, timeout=timeout))
    if raw(runs[0]) != raw(runs[1]):
        return ("error", "nondeterministic: two runs of one variant differ"), runs[0]
    return observe(runs[0], observers, table, live_level), runs[0]


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
        run = sandbox.run([una, "--json", "run", ".", *RUN_LIMITS], cwd=pkg, writable=rundir,
                          timeout=timeout)
        try:
            status = json.loads(run.stdout).get("status")
        except (ValueError, AttributeError):
            status = None
        if status in ("ok", "failed"):
            end = ("complete", None)
        elif status == "aborted":
            end = ("abort", None)
        else:
            continue
        native_check(result, una, files, name, root, timeout, end, run)


def oracle(rows, una, root, timeout, out, rebuild, plant=None, native_plant=None):
    """Runs the oracle on every manifest row with varied literals. `rebuild` turns a row into its
    program (`drivers.rebuild`); `plant`, for the gate's canaries only, rewrites the table of
    `accepts`; `native_plant`, for the native canary only, puts the program in the native sample and
    gives the files its executables are built from. Writes one result per program to `out` and
    returns the tally.

    A program's result counts `compared`, its complete-against-complete comparisons, and says
    whether it is `live`: whether two of its complete ends differ at the literal's own level, where
    every event below it is visible with its outcome."""
    tally = Counter()
    for row in rows:
        if not row.get("vary"):
            continue
        prog = rebuild(row)
        observers = observers_for(prog.vary)
        live_level = literal_level(prog.vary)
        result = {"name": prog.name, "op": prog.op, "seed": prog.seed, "edition": prog.edition,
                  "class": prog.cls, "observers": observers, "variants": [], "finding": False,
                  "error": None, "termination": [], "violations": [], "compared": 0,
                  "live": False,
                  "native": {"sampled": native_plant is not None or native.sampled(prog.name),
                             "stratum": False, "compared": 0, "divergences": [], "skipped": [],
                             "not_compiled": None}}
        if not observers:
            result["set_aside"] = "no observer below the varied literal"
        elif releases_to_public(prog.files):
            result["set_aside"] = "a policy releases to public"
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
        core, why = export_core(una, pkg, root, timeout)
        if core is None:
            # A program the checks reject has no complete export: the base run says so.
            base = run_twice(una, prog.files, prog.name + "-base", root, timeout, observers,
                             {k: {v} for k, v in HOST_ACCEPTS.items()}, live_level)
            if base[0] == "rejected":
                result["set_aside"] = "rejected by the checks: " + ", ".join(base[1])
                tally["set aside (rejected)"] += 1
            else:
                result["error"] = "base: " + why
                tally["program error"] += 1
            out.write(json.dumps(result, sort_keys=True) + "\n")
            continue
        table = accepts_table(core)
        if plant is not None:
            table = plant(table)
        base, base_run = run_twice_kept(una, prog.files, prog.name + "-base", root, timeout,
                                        observers, table, live_level)
        if base[0] == "rejected":
            result["error"] = "the checks reject a program whose Core export is complete"
        elif base[0] == "error":
            result["error"] = "base: " + base[1]
        native_check(result, una, prog.files, prog.name + "-base", root, timeout, base, base_run,
                     native_plant(prog.files) if native_plant else None)
        ref = base if base[0] == "complete" else None
        completes = [base] if base[0] == "complete" else []
        for i, pos in enumerate(prog.vary):
            if result["error"]:
                break
            for j, text in enumerate(values(pos["type"], pos["literal"], prog.files)):
                files = substitute(prog.files, pos, text)
                end, end_run = run_twice_kept(una, files, f"{prog.name}-v{i}-{j}", root, timeout,
                                              observers, table, live_level)
                native_check(result, una, files, f"{prog.name}-v{i}-{j}", root, timeout, end,
                             end_run, native_plant(files) if native_plant else None)
                if end[0] == "rejected":
                    end = ("error", "a variant is rejected by the checks: " + ", ".join(end[1]))
                verdict = judge(ref, end)
                entry = {"position": i, "value": text, "verdict": verdict}
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
