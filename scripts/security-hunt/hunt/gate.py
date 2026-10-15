"""The `hunt` release gate (docs/release-gates.md): a fixed, seeded run of the generators through
the must-reject and must-keep drivers and the two-run oracle, with two canaries.

It fails closed. The gate passes only when all of these hold:
- no must-reject program is accepted, no refactor turns a rejected program into an accepted one,
  and the oracle sees no difference an observer below the varied literal could see;
- every check and every run ends in a known way (no timeout of `una check`, no internal error,
  no unreadable output, no nondeterministic run);
- each driver judged exactly the cases the fixed run generates, and none of those counts is zero;
- both canaries are caught: a must-reject program that is in fact accepted, and a program whose
  printed public literal the manifest marks as secret.
"""

import io
import json
import os
import subprocess
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

from . import drivers, oracle
from .families import OPERATORS
from .program import MUST_ACCEPT, MUST_REJECT, OPEN, RESIDUAL, Program, derive_seed, rng_for
from .program import write_manifest
from .refactor import REFACTORS
from .una import Varied, header, join_decls, package

# The fixed run: changing any of these changes the gate, and docs/release-gates.md with it.
SEED = 767
COUNT = 50
EDITIONS = ("0.1", "0.2")
REFACTOR_IDS = ("R6", "R9")
# The fixed run's sizes, pinned: a change of the generators that changes them fails the gate
# until this table and docs/release-gates.md say so.
PINNED = {"programs": 5000, "reject": 5000, "keep": 8594, "oracle": 434}
# Floors on what the oracle compares, against a run that sets most programs aside.
MIN_ORACLE_COMPARED = 0
MIN_VARIANTS_COMPARED = 0
CHECK_TIMEOUT = 60
RUN_TIMEOUT = 20
WORKERS = 4

MOD = "app.main"
CONSOLE_MAIN = "pub main() -> Unit\n  uses @console: Console[print]\n=\n"


def programs():
    """Every program of the fixed run, in a fixed order."""
    out = []
    for op in sorted(OPERATORS):
        for i in range(COUNT):
            seed = derive_seed(SEED, op, i)
            for edition in EDITIONS:
                out.append(OPERATORS[op](seed, edition))
    return out


def canary_reject():
    """A must-reject row whose program the checks accept: the must-reject driver must call it a
    finding."""
    text = join_decls(header(MOD), CONSOLE_MAIN + '  @console.print("canary")')
    files = package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"])
    return Program(op="canary-reject", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=MUST_REJECT, files=files, expect=["U4001"])


def canary_oracle():
    """An accepted program that prints a public literal, which its manifest marks as a secret: its
    variants differ in public output, and the oracle must call it a leak."""
    v = Varied()
    lit = v.lit("Int", "secret", "5")
    text = join_decls(header(MOD), CONSOLE_MAIN + f"  let x: Int = {lit}\n"
                      '  @console.print("{x}")')
    files, vary = v.place(package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"]))
    return Program(op="canary-oracle", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=OPEN, files=files, vary=vary)


def expected_keep(progs):
    """How many (program, refactor) pairs the must-keep driver judges: the refactors that apply."""
    n = 0
    for p in progs:
        for rid in REFACTOR_IDS:
            if REFACTORS[rid](p.files, rng_for(derive_seed(p.seed, rid, 0))) is not None:
                n += 1
    return n


def _sharded(fn, rows, workers):
    """Runs `fn(rows_part, out, k)` on `workers` contiguous parts of `rows` at once, part `k` in a
    directory of its own, and returns the joined output lines in row order and the summed tally."""
    size = max(1, -(-len(rows) // workers))
    parts = [rows[i:i + size] for i in range(0, len(rows), size)]

    def one(k):
        buf = io.StringIO()
        return fn(parts[k], buf, k), buf.getvalue()

    with ThreadPoolExecutor(max_workers=workers) as pool:
        done = list(pool.map(one, range(len(parts))))
    tally = Counter()
    lines = []
    for t, text in done:
        tally.update(t)
        lines += [json.loads(line) for line in text.splitlines() if line.strip()]
    return tally, lines


def _version(una):
    try:
        p = subprocess.run([una, "--version"], capture_output=True, text=True, timeout=30)
        return p.stdout.strip()
    except (OSError, subprocess.SubprocessError) as e:
        return f"unknown ({e})"


def run(una, out_dir, tip):
    """Runs the gate. Writes OUT/manifest.jsonl, reject.jsonl, keep.jsonl, oracle.jsonl,
    canaries.jsonl and summary.json, prints the summary, and returns 0 (PASS) or 1 (FAIL)."""
    root = drivers.campaign_root(out_dir)
    progs = programs()
    by_name = {p.name: p for p in progs}
    if len(by_name) != len(progs):
        raise SystemExit("two programs of the fixed run share a name")
    write_manifest(os.path.join(root, "manifest.jsonl"), progs)
    rows = drivers.read_manifest(os.path.join(root, "manifest.jsonl"))
    work = os.path.join(root, "work")
    os.makedirs(work, exist_ok=True)

    def shard(k):
        """The writable directory of part `k`: concurrent runs never share a home or a package."""
        path = os.path.join(work, f"part{k}")
        os.makedirs(path, exist_ok=True)
        return path
    errors, findings, notes = [], [], []

    # Must-reject.
    judged = [r for r in rows if r["class"] in (MUST_REJECT, MUST_ACCEPT, OPEN, RESIDUAL)]
    reject_tally, reject_rows = _sharded(
        lambda part, buf, k: drivers.must_reject(part, una, shard(k), CHECK_TIMEOUT, buf), judged,
        WORKERS)
    for r in reject_rows:
        if r["verdict"] not in ("accepted", "rejected"):
            errors.append(f"reject {r['name']}: {r['verdict']} {r.get('detail', '')[:200]}")
        if r.get("finding"):
            findings.append(f"reject {r['name']}: a must-reject program is accepted")
        if r.get("over_strict"):
            notes.append(f"reject {r['name']}: a must-accept program is rejected")

    # Must-keep.
    keep_tally, keep_rows = _sharded(
        lambda part, buf, k: drivers.must_keep(part, una, shard(k), CHECK_TIMEOUT, buf,
                                                  REFACTOR_IDS),
        rows, WORKERS)
    for r in keep_rows:
        for side in ("base", "twin"):
            if r[side]["verdict"] not in ("accepted", "rejected"):
                errors.append(f"keep {r['name']} {r['refactor']} {side}: {r[side]['verdict']}")
        if r.get("direction") == "rejected-to-accepted":
            findings.append(f"keep {r['name']} {r['refactor']}: rejected to accepted")
        elif r.get("direction") == "accepted-to-rejected":
            notes.append(f"keep {r['name']} {r['refactor']}: accepted to rejected")

    # The oracle.
    varied = [r for r in rows if r.get("vary")]
    oracle_tally, oracle_rows = _sharded(
        lambda part, buf, k: oracle.oracle(part, una, shard(k), RUN_TIMEOUT, buf,
                                           drivers.rebuild),
        varied, WORKERS)
    termination = []
    for r in oracle_rows:
        if r.get("error"):
            errors.append(f"oracle {r['name']}: {r['error']}")
        if r.get("finding"):
            seen = sorted({o for v in r["variants"] for o in v.get("observers", [])})
            findings.append(f"oracle {r['name']}: an observer at {', '.join(seen)} sees the "
                            "varied literal")
        for t in r.get("termination", []):
            termination.append(dict(t, name=r["name"]))
    compared = sum(1 for r in oracle_rows for v in r["variants"] if v["verdict"] != "error")

    # Canaries.
    canaries = {}
    crej = canary_reject()
    buf = io.StringIO()
    drivers.must_reject([dict(crej.manifest(), digest=crej.digest())], una, work, CHECK_TIMEOUT,
                        buf, build=lambda row: crej)
    row = json.loads(buf.getvalue())
    canaries["reject"] = bool(row.get("finding"))
    cor = canary_oracle()
    buf = io.StringIO()
    oracle.oracle([dict(cor.manifest(), digest=cor.digest())], una, work, RUN_TIMEOUT, buf,
                  lambda row: cor)
    row = json.loads(buf.getvalue())
    canaries["oracle"] = bool(row.get("finding")) and not row.get("error")
    with open(os.path.join(root, "canaries.jsonl"), "w", encoding="utf-8") as f:
        f.write(json.dumps(canaries, sort_keys=True) + "\n")

    for name, lines in (("reject", reject_rows), ("keep", keep_rows), ("oracle", oracle_rows)):
        with open(os.path.join(root, f"{name}.jsonl"), "w", encoding="utf-8") as f:
            for line in lines:
                f.write(json.dumps(line, sort_keys=True) + "\n")

    sizes = {
        "programs": len(progs),
        "reject": {"expected": len(judged), "judged": len(reject_rows)},
        "keep": {"expected": expected_keep(progs), "judged": len(keep_rows)},
        "oracle": {"expected": len(varied), "judged": len(oracle_rows),
                   "set_aside": sum(1 for r in oracle_rows if r.get("set_aside")),
                   "variants_compared": compared},
    }
    for key in ("reject", "keep", "oracle"):
        s = sizes[key]
        if s["judged"] != s["expected"]:
            errors.append(f"{key}: judged {s['judged']} of {s['expected']} cases")
        if s["judged"] == 0:
            errors.append(f"{key}: no case judged")
    if compared == 0:
        errors.append("oracle: no variant compared")
    if compared < MIN_VARIANTS_COMPARED:
        errors.append(f"oracle: {compared} variants compared, under the floor of "
                      f"{MIN_VARIANTS_COMPARED}")
    programs_compared = sum(1 for r in oracle_rows if not r.get("set_aside"))
    if programs_compared < MIN_ORACLE_COMPARED:
        errors.append(f"oracle: {programs_compared} programs compared, under the floor of "
                      f"{MIN_ORACLE_COMPARED}")
    for key, want in PINNED.items():
        got = sizes[key] if key == "programs" else sizes[key]["expected"]
        if got != want:
            errors.append(f"{key}: the fixed run has {got} cases, and the gate pins {want}")
    for name, caught in sorted(canaries.items()):
        if not caught:
            errors.append(f"canary {name}: not caught, so the gate cannot see that fault")

    verdict = "PASS" if not errors and not findings else "FAIL"
    summary = {
        "verdict": verdict, "seed": SEED, "count_per_operator": COUNT,
        "editions": list(EDITIONS), "refactors": list(REFACTOR_IDS), "tip": tip,
        "una": _version(una), "sizes": sizes, "canaries": canaries,
        "findings": findings, "errors": errors,
        "termination_pairs": {"count": len(termination), "pairs": termination},
        "notes": notes,
        "tallies": {
            "reject": {" ".join(map(str, k)): n for k, n in sorted(reject_tally.items())},
            "keep": {" ".join(map(str, k)): n for k, n in sorted(keep_tally.items())},
            "oracle": dict(sorted(oracle_tally.items())),
        },
    }
    with open(os.path.join(root, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=1, sort_keys=True)
        f.write("\n")
    print(f"hunt gate: seed {SEED}, {len(progs)} programs, una tip {tip}")
    for key in ("reject", "keep", "oracle"):
        print(f"  {key}: {sizes[key]['judged']} of {sizes[key]['expected']} cases judged")
    print(f"  oracle: {compared} variants compared, {sizes['oracle']['set_aside']} programs set aside")
    print(f"  canaries: " + ", ".join(f"{k} {'caught' if v else 'MISSED'}"
                                      for k, v in sorted(canaries.items())))
    print(f"  termination pairs (outside T6, recorded): {len(termination)}")
    for t in termination:
        print(f"    {t['name']} value {t['value']}: {t['base']} vs {t['variant']}")
    print(f"  notes (over-strict, not failures): {len(notes)}")
    for line in findings:
        print("  FINDING " + line)
    for line in errors:
        print("  ERROR " + line)
    print(f"hunt gate: {verdict}")
    return 0 if verdict == "PASS" else 1
