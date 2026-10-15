"""The two-run oracle (docs/plan/security-hunt-oracle.md), the part the release gate runs.

For a program `una check` accepts, with varied literals (`Program.vary`), each literal is
replaced by other values of its type, one position at a time, and every variant runs on the
interpreter, `una run --json`, twice. What a public observer sees (`project`) must be the same
in every variant (T6, `spec/07 §1`). Scope: V-secret only (source literals behind a label above
`public`), the public observer, the interpreter. V-host, the Http mock, the final tree of
`Files.write` and the native sample are not part of it.

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

from . import sandbox
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


def project(doc):
    """What a public observer sees of one run (docs/release-gates.md, the `hunt` gate):
    - `status`, every `output` line in order, and `failure` (its variant and its public fields);
    - `diagnostics`, by severity and code;
    - every `trace` event, all its fields but `outcome`: the host's answer, an input of the run,
      not something the program sends (`spec/07 §1` lists operation arguments as the channel).
    """
    return {
        "status": doc.get("status"),
        "output": doc.get("output"),
        "failure": doc.get("failure"),
        "diagnostics": [[d.get("severity"), d.get("code")] for d in doc.get("diagnostics", [])],
        "trace": [{k: v for k, v in ev.items() if k != "outcome"} for ev in doc.get("trace", [])],
    }


def observe(run):
    """One run's end: (`complete`, view), (`abort`, codes), (`rejected`, codes), or
    (`error`, why)."""
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
    codes = sorted({d.get("code") for d in doc.get("diagnostics", [])
                    if d.get("severity") == "error"})
    if status == "rejected" and run.code == 1:
        return ("rejected", codes)
    if status == "ok" and run.code == 0:
        if doc.get("diagnostics"):
            return ("error", "an ok run with diagnostics: "
                             + ", ".join(d.get("code", "?") for d in doc["diagnostics"]))
        return ("complete", project(doc))
    if status == "failed" and run.code == 1 and doc.get("failure"):
        return ("complete", project(doc))
    if status == "aborted" and run.code == 1 and codes and set(codes) <= ABORT_CODES:
        return ("abort", codes)
    return ("error", f"status {status!r}, exit {run.code}, codes {codes}")


def judge(base, other):
    """The verdict on two variants' ends: `same`, `leak`, `termination` or `error`."""
    if base[0] == "error" or other[0] == "error":
        return "error"
    if base[0] == "complete" and other[0] == "complete":
        return "same" if base[1] == other[1] else "leak"
    if base[0] == "abort" and other[0] == "abort":
        return "same"
    if {base[0], other[0]} == {"complete", "abort"}:
        return "termination"
    return "error"


def run_twice(una, files, name, root, timeout):
    """Writes the variant, runs it twice, and returns its end, or an error when the two runs
    differ."""
    p = Program(op="oracle", rule="", edition="", seed=0, cls="open", files=files, variant=name)
    pkg = p.write(root)
    ends = []
    for _ in range(2):
        r = sandbox.run([una, "--json", "run", ".", *RUN_LIMITS], cwd=pkg, writable=root,
                        timeout=timeout)
        ends.append(observe(r))
    if ends[0] != ends[1]:
        return ("error", "nondeterministic: two runs of one variant differ")
    return ends[0]


def oracle(rows, una, root, timeout, out, rebuild):
    """Runs the oracle on every manifest row with varied literals. `rebuild` turns a row into its
    program (`drivers.rebuild`). Writes one result per program to `out` and returns the tally."""
    tally = Counter()
    for row in rows:
        if not row.get("vary"):
            continue
        prog = rebuild(row)
        result = {"name": prog.name, "op": prog.op, "seed": prog.seed, "edition": prog.edition,
                  "class": prog.cls, "variants": [], "finding": False, "error": None,
                  "termination": []}
        if releases_to_public(prog.files):
            result["set_aside"] = "a policy releases to public"
            tally["set aside (policy)"] += 1
            out.write(json.dumps(result, sort_keys=True) + "\n")
            continue
        base = run_twice(una, prog.files, prog.name + "-base", root, timeout)
        if base[0] == "rejected":
            result["set_aside"] = "rejected by the checks: " + ", ".join(base[1])
            tally["set aside (rejected)"] += 1
            out.write(json.dumps(result, sort_keys=True) + "\n")
            continue
        if base[0] == "error":
            result["error"] = "base: " + base[1]
        for i, pos in enumerate(prog.vary):
            if result["error"]:
                break
            for j, text in enumerate(values(pos["type"], pos["literal"], prog.files)):
                files = substitute(prog.files, pos, text)
                end = run_twice(una, files, f"{prog.name}-v{i}-{j}", root, timeout)
                verdict = judge(base, end)
                if end[0] == "rejected":
                    verdict = "error"
                    end = ("error", "a variant is rejected by the checks: " + ", ".join(end[1]))
                result["variants"].append({"position": i, "value": text, "verdict": verdict})
                tally["variant " + verdict] += 1
                if verdict == "leak":
                    result["finding"] = True
                elif verdict == "termination":
                    result["termination"].append(
                        {"value": text, "base": base[0] if base[0] == "complete" else base[1],
                         "variant": end[0] if end[0] == "complete" else end[1]})
                elif verdict == "error":
                    result["error"] = end[1] if end[0] == "error" else base[1] if \
                        base[0] == "error" else f"ends {base[0]} and {end[0]}"
                    break
        tally["program " + ("error" if result["error"] else
                            "finding" if result["finding"] else "compared")] += 1
        out.write(json.dumps(result, sort_keys=True) + "\n")
        out.flush()
    return tally

