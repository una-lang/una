"""The must-reject and must-keep drivers (docs/plan/security-hunt-generators.md §1).

Both judge with `una check --tier 0` alone: the label rules are tier 0, and the solver's
obligations are not what they test. Every run goes through `sandbox.run`.
"""

import json
import os
from collections import Counter

from . import sandbox
from .families import OPERATORS
from .program import MUST_ACCEPT, MUST_REJECT, OPEN, RESIDUAL, Program, derive_seed, rng_for
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


def must_reject(rows, una, root, timeout, out, build=None):
    """Checks every must-reject program; an accepted one is a finding. Open programs are
    checked too, and their verdict is recorded for triage. `build` turns a row into its program
    (`rebuild` by default; the gate's canaries pass their own)."""
    tally = Counter()
    for row in rows:
        if row["class"] not in (MUST_REJECT, MUST_ACCEPT, OPEN, RESIDUAL):
            continue
        prog = (build or rebuild)(row)
        v = check(una, prog.write(root), root, timeout)
        result = dict(row, **v.row())
        if row["class"] == MUST_REJECT:
            result["finding"] = v.kind == "accepted"
            got = {c[0] for c in v.codes}
            result["expect_seen"] = bool(got & set(row["expect"])) if row["expect"] else None
        elif row["class"] == MUST_ACCEPT:
            result["finding"] = False
            result["over_strict"] = v.kind != "accepted"
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


# The re-check's note for F8's two expected outcomes other than "accepted" (#548).
UNKNOWN_OPERANDS = "on operands of mixed or unknown integer types"
FIXED_PRIMITIVE = "the primitive `fixed`"


def judge_differ(exp, core_text, rec, main=None, expect_main=""):
    """Part (b)'s verdict on one program `una check` accepted, fail-closed (N-99's rule, Una-2's
    review of #527): `finding` when the re-check rejects a declaration, `clean` only when the
    export is complete and read, the re-check ran to its end, and every function of the export
    has its line; anything else is `unknown`, with why, and blocks "no finding".

    `exp` and `rec` are `sandbox.Run`s of `una --json core` and of `una-core-check`, whose exit
    code is 0 when every document is read and no declaration is rejected, 1 otherwise
    (`formal/Recheck.lean`)."""
    rows = parse_recheck(rec.stdout) if rec is not None else []
    out = {"kind": "unknown", "detail": "", "recheck": rows}
    if exp.timed_out or exp.code != 0:
        out["detail"] = "the export timed out" if exp.timed_out else f"the export exited {exp.code}"
        return out
    try:
        doc = json.loads(core_text)
    except ValueError:
        out["detail"] = "the export is not JSON"
        return out
    if doc.get("complete") is not True:
        out["detail"] = "the export is not complete"
        return out
    outcomes = [r["outcome"] for r in rows]
    if "rejected" in outcomes:
        # A rejection the re-check printed is a finding even if it then timed out or crashed
        # (Una-2's review of #527): the abnormal end goes into the detail, never hides it.
        out["kind"] = "finding"
        if rec is not None and (rec.timed_out or rec.code not in (0, 1)):
            out["detail"] = ("a rejected declaration, then the re-check timed out" if rec.timed_out
                             else f"a rejected declaration, then the re-check exited {rec.code}")
        return out
    if rec is None or rec.timed_out or rec.code not in (0, 1):
        out["detail"] = ("the re-check did not run" if rec is None else
                         "the re-check timed out" if rec.timed_out else
                         f"the re-check exited {rec.code}")
        return out
    if "error" in outcomes or any(r["decl"] == "-" for r in rows):
        out["detail"] = "the re-check could not read the document"
        return out
    if rec.code != 0:
        out["detail"] = "the re-check exited 1 without a rejected declaration"
        return out
    # Every function and every refinement predicate has its line (#532): a predicate is reported
    # outside the flow model, never dropped.
    funs = {f"{m.get('path')}.{d['value']['name']}" for m in doc.get("modules", [])
            for d in m.get("decls", []) if d.get("kind") == "fun"}
    preds = {f"{m.get('path')}.{d.get('name', '?')}" for m in doc.get("modules", [])
             for d in m.get("decls", []) if d.get("kind") == "pred"}
    funs |= preds
    missing = sorted(funs - {r["decl"] for r in rows})
    if missing:
        out["detail"] = f"the re-check has no line for {', '.join(missing[:3])}"
        return out
    # A predicate's line says exactly "outside the flow model" (Una-2's review of #540), unless
    # the whole document fell back to one outcome: every line "not re-checked" for one reason
    # (`checkDocument`'s fallbacks, `docs/plan/formal-track-status.md`).
    fallback = (bool(rows) and all(r["outcome"] == "not re-checked" for r in rows)
                and len({r["note"] for r in rows}) == 1)
    if not fallback:
        odd_preds = sorted(r["decl"] for r in rows
                           if r["decl"] in preds and r["outcome"] != "outside the flow model")
        if odd_preds:
            out["detail"] = f"the predicate {odd_preds[0]} is not outside the flow model"
            return out
    known = {"accepted", "accepted, conditional on A3", "not re-checked", "outside the flow model"}
    odd = sorted(set(outcomes) - known)
    if odd:
        out["detail"] = f"the re-check printed the outcome {odd[0]!r}"
        return out
    # F8 (N-138): what the re-check says of the program's own declaration is part of the verdict.
    # "not-rechecked" is clean only when it is not re-checked for its operands' unknown types, and
    # "outside" is `unknown`, out of F1, by design; either accepted is a finding (fail closed).
    if main is not None and expect_main in ("not-rechecked", "outside"):
        line = next((r for r in rows if r["decl"] == main), None)
        if line is None:
            out["detail"] = f"the re-check has no line for {main}"
            return out
        if line["outcome"] != "not re-checked":
            out["kind"] = "finding"
            out["detail"] = f"{main} is {line['outcome']!r}, but must not be re-checked ({expect_main})"
            return out
        want = UNKNOWN_OPERANDS if expect_main == "not-rechecked" else FIXED_PRIMITIVE
        if want not in line["note"]:
            out["detail"] = f"{main} is not re-checked for another reason: {line['note']}"
            return out
        if expect_main == "outside":
            out["detail"] = f"out of F1 (expected): {line['note']}"
            out["expected"] = True
            return out
        out["kind"] = "clean"
        return out
    # Family F stays inside the re-check's vocabulary by design (Una-2's review of #547): its
    # program's own declaration not re-checked is not a comparison, so it is `unknown`.
    if main is not None and any(r["decl"] == main and r["outcome"] == "not re-checked"
                                for r in rows):
        out["detail"] = f"the program's declaration {main} is not re-checked"
        return out
    out["kind"] = "clean"
    return out


def differ(rows, una, core_check, root, timeout, out, z3=None):
    """Part (b): for each program `una check` accepts, exports its Core and runs the re-check on
    it, and judges with `judge_differ`. A `finding` goes to the coordinator before any
    interpretation; an `unknown` blocks the campaign's "no finding" until it is understood.
    Programs `una check` rejects are recorded only: the reverse direction is #514's."""
    tally = Counter()
    for row in rows:
        prog = rebuild(row)
        pkg = prog.write(root)
        v = check(una, pkg, root, timeout)
        # The class is the generator's own, so a manifest written before a program was classified
        # is judged by its class today (Una-2's review of #547).
        result = dict(row, **v.row(), **{"class": prog.cls, "expect": prog.expect})
        result["differ"] = None
        result["finding"] = False
        # `una check` alone decides a must-reject or must-accept program, whatever the re-check says.
        if prog.cls == MUST_REJECT:
            result["finding"] = v.kind == "accepted"
            got = {c[0] for c in v.codes}
            result["expect_seen"] = bool(got & set(prog.expect)) if prog.expect else None
            tally[(row["op"], MUST_REJECT, v.kind)] += 1
        elif prog.cls == MUST_ACCEPT:
            result["over_strict"] = v.kind != "accepted"
            tally[(row["op"], MUST_ACCEPT, v.kind)] += 1
        elif prog.cls == RESIDUAL:
            # Descriptive (N-138): both tiers' verdicts are recorded. The full check is where the
            # `overflow` and `division` obligations are decided, the guard of this channel, so it
            # needs the solver (`judge_residual`).
            fv = None
            if z3:
                fv = parse_check(sandbox.run([una, "--json", "check", "."], cwd=pkg, writable=root,
                                             timeout=timeout, env_extra=solver_env(z3)))
            r = judge_residual(v.kind, fv, prog.variant)
            result.update(full=None if fv is None else fv.row(), **r)
            tally[(row["op"], RESIDUAL, "tier0 " + v.kind, "full " + r["full_kind"])] += 1
        if v.kind == "accepted":
            exp = sandbox.run([una, "--json", "core", "."], cwd=pkg, writable=root,
                              timeout=timeout)
            core = os.path.join(pkg, "core.json")
            with open(core, "w", encoding="utf-8") as f:
                f.write(exp.stdout)
            rec = None
            if not exp.timed_out and exp.code == 0:
                rec = sandbox.run([core_check, core], cwd=pkg, writable=root, timeout=timeout)
            main = "app.main.run" if row["op"].startswith("F") else None
            j = judge_differ(exp, exp.stdout, rec, main=main, expect_main=prog.recheck)
            result.update(differ=j["kind"], differ_detail=j["detail"], recheck=j["recheck"],
                          finding=result["finding"] or j["kind"] == "finding",
                          export_code=exp.code, export_timed_out=exp.timed_out,
                          recheck_code=None if rec is None else rec.code,
                          recheck_timed_out=None if rec is None else rec.timed_out)
            if prog.cls == RESIDUAL:
                # Never a clean comparison, in the rows file as in the tally (N-146): the
                # re-check's own verdict stays in `differ_raw`.
                result.update(differ="residual", differ_raw=j["kind"])
            if j["kind"] == "unknown" and not j.get("expected"):
                result["bug"] = j["detail"]
            result["differ_expected"] = bool(j.get("expected"))
            kind = j["kind"] + (" (expected)" if j.get("expected") else "")
            if prog.cls == RESIDUAL:
                # Never a clean must-accept: the comparison is recorded under its own class.
                kind = "residual " + j["kind"]
            tally[(row["op"], "una-accepted", kind)] += 1
            for x in j["recheck"]:
                tally[(row["op"], "declaration", x["outcome"].split(",")[0])] += 1
        else:
            tally[(row["op"], v.kind)] += 1
        out.write(json.dumps(result, sort_keys=True) + "\n")
        out.flush()
    return tally


# The full check's codes that decide a residual program: its obligation refuted or unknown.
OBLIGATION_CODES = {"U5001", "U5002"}


def judge_residual(tier0, full, variant):
    """A residual program's verdicts (N-138, N-146), fail-closed. `tier0` is `una check --tier 0`'s
    kind, `full` the full check's `Verdict`, or None when no solver was given. The row is invalid,
    and the batch is rerun, unless the full check ran with the solver to an acceptance or to a
    rejection by an obligation: no solver, U5003, a timeout, an ICE, a usage error, an unreadable
    or inconsistent output, or a rejection by other codes, which tests nothing of the channel. A
    public print after the step goes to triage when either check accepts it."""
    after = variant.endswith("-print-after")
    if full is None:
        kind = "invalid (no solver)"
    elif any(c[0] == "U5003" for c in full.codes):
        kind = "invalid (U5003)"
    elif full.kind not in ("accepted", "rejected"):
        kind = f"invalid ({full.kind})"
    elif full.kind == "rejected" and not {c[0] for c in full.codes} <= OBLIGATION_CODES:
        kind = "invalid (rejected by " + ", ".join(sorted({c[0] for c in full.codes})) + ")"
    else:
        kind = full.kind
    return {"full_kind": kind, "invalid": kind.startswith("invalid"),
            "triage": kind == "accepted" and after, "triage_tier0": tier0 == "accepted" and after}


def solver_env(z3):
    """The `PATH` for a full check: the directory of the solver `z3` first, then the sandbox's
    own."""
    return {"PATH": os.path.dirname(os.path.realpath(z3)) + ":/usr/bin:/bin"}


def read_manifest(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def campaign_root(path):
    root = os.path.realpath(path)
    if root in ("/", os.path.realpath(os.path.expanduser("~"))):
        raise ValueError("the campaign root must be its own directory")
    os.makedirs(root, exist_ok=True)
    return root
