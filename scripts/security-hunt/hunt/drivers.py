"""The must-reject and must-keep drivers (docs/plan/security-hunt-generators.md §1).

Both judge with `una check --tier 0` alone: the label rules are tier 0, and the solver's
obligations are not what they test. Every run goes through `sandbox.run`.
"""

import json
import os
from collections import Counter

from . import sandbox
from .families import OPERATORS
from .program import MUST_REJECT, OPEN, Program, derive_seed, rng_for
from .refactor import REFACTORS


class Verdict:
    """`una check`'s verdict on one package: accepted, rejected, ice, usage or timeout."""

    def __init__(self, kind, codes, seconds, detail=""):
        self.kind = kind
        self.codes = codes
        self.seconds = seconds
        self.detail = detail

    def row(self):
        return {"verdict": self.kind, "codes": self.codes, "seconds": round(self.seconds, 3),
                "detail": self.detail}


def parse_check(run):
    """Reads `una --json check`'s output (`spec/13 §1`, §5)."""
    if run.timed_out:
        return Verdict("timeout", [], run.seconds)
    if run.code == 3:
        return Verdict("ice", [], run.seconds, run.stderr[-2000:])
    if run.code == 2:
        return Verdict("usage", [], run.seconds, run.stderr[-2000:])
    try:
        doc = json.loads(run.stdout)
    except ValueError:
        return Verdict("unreadable", [], run.seconds, run.stdout[-2000:] + run.stderr[-2000:])
    errors = [d for d in doc.get("diagnostics", []) if d.get("severity") == "error"]
    codes = sorted(
        [d["code"], d["primary"]["file"], d["primary"]["start"]["line"],
         d["primary"]["start"]["col"]] for d in errors)
    if run.code == 0 and not errors:
        return Verdict("accepted", [], run.seconds)
    if run.code == 1 and errors:
        return Verdict("rejected", codes, run.seconds)
    return Verdict("inconsistent", codes, run.seconds, f"exit {run.code}")


def check(una, pkg_dir, root, timeout):
    r = sandbox.run([una, "--json", "check", "--tier", "0", "."], cwd=pkg_dir, writable=root,
                    timeout=timeout)
    return parse_check(r)


def rebuild(row):
    """The program of a manifest row, rebuilt from its operator, seed and edition. A digest
    that differs means the generator changed since the manifest was written."""
    prog = OPERATORS[row["op"]](row["seed"], row["edition"])
    if row.get("digest") and prog.digest() != row["digest"]:
        raise ValueError(f"{row['name']}: the rebuilt program differs from the manifest")
    return prog


def must_reject(rows, una, root, timeout, out):
    """Checks every must-reject program; an accepted one is a finding. Open programs are
    checked too, and their verdict is recorded for triage."""
    tally = Counter()
    for row in rows:
        if row["class"] not in (MUST_REJECT, OPEN):
            continue
        prog = rebuild(row)
        v = check(una, prog.write(root), root, timeout)
        result = dict(row, **v.row())
        if row["class"] == MUST_REJECT:
            result["finding"] = v.kind == "accepted"
            got = {c[0] for c in v.codes}
            result["expect_seen"] = bool(got & set(row["expect"])) if row["expect"] else None
        else:
            result["finding"] = False
        if v.kind in ("ice", "inconsistent", "unreadable"):
            result["bug"] = v.kind
        tally[(row["op"], row["class"], v.kind)] += 1
        out.write(json.dumps(result, sort_keys=True) + "\n")
        out.flush()
    return tally


def must_keep(rows, una, root, timeout, out, refactors=("R6", "R9")):
    """Applies each refactor to each program, and compares the two verdicts: a change from
    rejected to accepted is a candidate leak, the reverse an over-strict checker."""
    tally = Counter()
    for row in rows:
        prog = rebuild(row)
        base = check(una, prog.write(root), root, timeout)
        for rid in refactors:
            rseed = derive_seed(prog.seed, rid, 0)
            files = REFACTORS[rid](prog.files, rng_for(rseed))
            if files is None:
                tally[(rid, "n/a")] += 1
                continue
            twin = Program(op=prog.op, rule=prog.rule, edition=prog.edition, seed=prog.seed,
                           cls=prog.cls, files=files, expect=prog.expect,
                           variant=(prog.variant + "-" if prog.variant else "") + rid)
            v = check(una, twin.write(root), root, timeout)
            changed = (base.kind == "accepted") != (v.kind == "accepted")
            result = {
                "name": prog.name, "op": prog.op, "seed": prog.seed, "edition": prog.edition,
                "refactor": rid, "refactor_seed": rseed,
                "base": base.row(), "twin": v.row(), "finding": changed,
                "direction": ("rejected-to-accepted" if v.kind == "accepted" else
                              "accepted-to-rejected") if changed else None,
                "codes_changed": sorted(c[0] for c in base.codes) != sorted(c[0] for c in v.codes),
            }
            tally[(rid, "changed" if changed else "kept")] += 1
            out.write(json.dumps(result, sort_keys=True) + "\n")
            out.flush()
    return tally


def parse_recheck(stdout):
    """Reads `una-core-check`'s lines (`formal/Recheck.lean`): path, declaration, outcome, note."""
    rows = []
    for line in stdout.splitlines():
        parts = line.split("\t", 3)
        if len(parts) >= 3:
            rows.append({"decl": parts[1], "outcome": parts[2],
                         "note": parts[3] if len(parts) > 3 else ""})
    return rows


def differ(rows, una, core_check, root, timeout, out):
    """Part (b): for each program `una check` accepts, exports its Core and runs the re-check on
    it. A declaration the re-check rejects is a candidate (`finding`), for the coordinator before
    any interpretation; `not re-checked` is recorded, and says which form the model lacks.
    Programs `una check` rejects are recorded only: the reverse direction is #514's."""
    tally = Counter()
    for row in rows:
        prog = rebuild(row)
        pkg = prog.write(root)
        v = check(una, pkg, root, timeout)
        result = dict(row, **v.row())
        result["recheck"] = []
        result["finding"] = False
        if v.kind == "accepted":
            exp = sandbox.run([una, "--json", "core", "."], cwd=pkg, writable=root,
                              timeout=timeout)
            core = os.path.join(pkg, "core.json")
            with open(core, "w", encoding="utf-8") as f:
                f.write(exp.stdout)
            r = sandbox.run([core_check, core], cwd=pkg, writable=root, timeout=timeout)
            rc = parse_recheck(r.stdout)
            result["recheck"] = rc
            result["export_code"] = exp.code
            outcomes = {x["outcome"] for x in rc}
            result["finding"] = "rejected" in outcomes
            if not rc:
                result["bug"] = "no re-check output"
            for x in rc:
                tally[(row["op"], "una-accepted", x["outcome"].split(",")[0])] += 1
        else:
            tally[(row["op"], v.kind)] += 1
        out.write(json.dumps(result, sort_keys=True) + "\n")
        out.flush()
    return tally


def read_manifest(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def campaign_root(path):
    root = os.path.realpath(path)
    if root in ("/", os.path.realpath(os.path.expanduser("~"))):
        raise ValueError("the campaign root must be its own directory")
    os.makedirs(root, exist_ok=True)
    return root
