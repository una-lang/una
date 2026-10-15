"""The `hunt` release gate (docs/release-gates.md): a fixed, seeded run of the generators through
the must-reject and must-keep drivers, the two-run oracle and the comparison with the model's
re-check (`differ`), with five canaries.

It fails closed. The gate passes only when all of these hold:
- no must-reject program is accepted, no refactor turns a rejected program into an accepted one,
  and the oracle sees no difference an observer below the varied literal could see;
- every check and every run ends in a known way (no timeout of `una check`, no internal error,
  no unreadable output, no nondeterministic run);
- the re-check rejects no declaration the checks accept; its `unknown` outcomes are exactly the
  pinned ones (`PINNED_UNKNOWN`), its residual comparisons the pinned count, and the programs it
  reads clean meet their floor;
- each driver judged exactly the cases the fixed run generates, and none of those counts is zero;
  the oracle's complete-against-complete comparisons, the programs that have one, and the programs
  whose variants differ at the literal's own level (liveness) meet their floors;
- every canary is caught (`CANARIES`): a must-reject program that is in fact accepted; a program
  whose printed public literal the manifest marks as secret; an operation planted as accepting
  `internal`, reached under a secret branch, which only the `internal` observer must see; and an
  operation planted as accepting `public`, sent a secret, whose redacted argument the oracle must
  report by itself; and a planted Core document the re-check must reject (`DIFFER_CANARY`).
"""

import io
import json
import os
import subprocess
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

from . import drivers, oracle, sandbox
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
PINNED = {"programs": 5000, "reject": 5000, "keep": 8594, "oracle": 434, "differ": 5000}
# The re-check comparison's expected `unknown` outcomes, by program, each with its reason: any
# other `unknown` is an error, and so is a pinned one that is no longer unknown (#791).
PINNED_UNKNOWN = {
    "F8-0.2-17ae8812afd2af8b-wrapping-secret-result": "out of F1 (expected): the primitive `fixed`",
    "F8-0.2-7c392c87affef562-saturating-secret-result": "out of F1 (expected): the primitive `fixed`",
    "F8-0.2-8fcd886c54b655f1-saturating-secret-result": "out of F1 (expected): the primitive `fixed`",
    "F8-0.2-f3c36fb985e9ab24-bit-secret-branch": "out of F1 (expected): the primitive `fixed`",
}
# The residual programs the re-check compares (a step a secret may abort, which the full check's
# obligations guard): their count is pinned, so a silent drop fails.
PINNED_RESIDUAL = 9
# A floor on the programs the re-check reads clean: about 90% of the measured run's 1,198.
MIN_DIFFER_CLEAN = 1078
# The planted Core document the re-check must reject: v0.3.14's export of a program whose
# `to_text` temporary it labeled `public` (S-27, docs/reviews/formal-m2-replay).
DIFFER_CANARY = "docs/reviews/formal-m2-replay/core-s27/S-27.interpolated_record.v0.3.14.json"
# Floors on what the oracle compares, against a run that sets most programs aside or compares
# literals that decide nothing: about 90% of the measured run's 334 programs and 1,202
# complete-against-complete comparisons, and of its 100 live programs (docs/release-gates.md).
MIN_ORACLE_COMPARED = 300
MIN_VARIANTS_COMPARED = 1000
MIN_LIVE = 90
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


VAULT = ("capability Vault\n  op note(x: Int) -> Unit\n    accepts secret\n\n"
         "handler quiet() for Vault\n  op note(x) = unit")


def canary_internal():
    """An accepted program that calls an operation accepting `secret` under a secret branch; the
    gate plants that operation as accepting `internal`. The `public` observer must see the same in
    every variant, and the `internal` observer the event come and go: a leak at `internal` only."""
    v = Varied()
    lit = v.lit("Bool", "secret", "true")
    text = join_decls(
        header(MOD), VAULT,
        "work(n: Int) -> Unit\n  uses @vault: Vault[note]\n=\n  @vault.note(n)",
        "guarded(flag: secret Bool) -> Unit\n=\n"
        "  if flag\n    work(1) with @vault = quiet()\n  else\n    unit",
        CONSOLE_MAIN + f"  let s: secret Bool = {lit}\n  guarded(s)\n"
        '  @console.print("done")')
    files, vary = v.place(package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"]))
    return Program(op="canary-internal", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=OPEN, files=files, vary=vary)


def canary_redacted():
    """An accepted program that sends a secret to an operation accepting `secret`; the gate plants
    that operation as accepting `public`. Every variant shows the same `<redacted:secret>`
    argument, so only the redacted-sink rule can report it."""
    v = Varied()
    lit = v.lit("Int", "secret", "5")
    text = join_decls(
        header(MOD), VAULT,
        "send(n: secret Int) -> Unit\n  uses @vault: Vault[note]\n=\n  @vault.note(n)",
        CONSOLE_MAIN + f"  let s: secret Int = {lit}\n  send(s) with @vault = quiet()\n"
        '  @console.print("done")')
    files, vary = v.place(package("canary", "0.2", {MOD: text}, entry=MOD, grants=["console"]))
    return Program(op="canary-redacted", rule="the gate's own canary", edition="0.2", seed=0,
                   cls=OPEN, files=files, vary=vary)


def planted(level):
    """The canaries' planted fault: `Vault.note` accepts `level` in the oracle's table."""
    def plant(table):
        table = dict(table)
        table[("Vault", "note")] = {level}
        return table
    return plant


def oracle_canary(prog, una, work, plant=None):
    """Runs one program through the oracle, and returns its result row."""
    buf = io.StringIO()
    oracle.oracle([dict(prog.manifest(), digest=prog.digest())], una, work, RUN_TIMEOUT, buf,
                  lambda row: prog, plant=plant)
    return json.loads(buf.getvalue())


def caught(name, row):
    """Whether a canary's result is the fault it plants, and only that."""
    if row.get("error") or row.get("set_aside") or not row.get("finding"):
        return False
    leaks = sorted({o for v in row["variants"] for o in v.get("observers", [])})
    if name == "oracle":
        return "public" in leaks
    if name == "internal":
        return leaks == ["internal"] and not row["violations"]
    if name == "redacted":
        return bool(row["violations"]) and not leaks
    return False


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


def differ_canary(core_check, work):
    """Whether the re-check comparison sees a planted fault: v0.3.14's Core export of a program
    whose `to_text` temporary it labeled `public` while it held a `secret[PII]` field (S-27). Read
    through `judge_differ` as a program's export, it must be a finding."""
    repo = os.path.realpath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
    path = os.path.join(repo, DIFFER_CANARY)
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return False
    exported = sandbox.Run(0, text, "", False, 0.0)
    rec = sandbox.run([core_check, path], cwd=work, writable=work, timeout=CHECK_TIMEOUT)
    return drivers.judge_differ(exported, text, rec)["kind"] == "finding"


def _version(una):
    try:
        p = subprocess.run([una, "--version"], capture_output=True, text=True, timeout=30)
        return p.stdout.strip()
    except (OSError, subprocess.SubprocessError) as e:
        return f"unknown ({e})"


def run(una, out_dir, tip, core_check, z3):
    """Runs the gate. Writes OUT/manifest.jsonl, reject.jsonl, keep.jsonl, oracle.jsonl,
    canaries.jsonl and summary.json, prints the summary, and returns 0 (PASS) or 1 (FAIL)."""
    if os.path.exists(out_dir) and os.listdir(out_dir):
        raise SystemExit(f"hunt gate: {out_dir} is not empty; the gate writes a fresh directory")
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
            if seen:
                findings.append(f"oracle {r['name']}: an observer at {', '.join(seen)} sees the "
                                "varied literal")
            for v in r.get("violations", []):
                findings.append(f"oracle {r['name']}: {v}")
        for t in r.get("termination", []):
            termination.append(dict(t, name=r["name"]))
    # Only complete-against-complete comparisons count toward the floors.
    compared = sum(r.get("compared", 0) for r in oracle_rows)
    programs_compared = sum(1 for r in oracle_rows if r.get("compared", 0) > 0)
    live = sum(1 for r in oracle_rows if r.get("live"))

    # The re-check comparison (part (b) of docs/plan/security-hunt.md, #791): every program the
    # checks accept is exported and re-checked by the model's checker.
    differ_tally, differ_rows = _sharded(
        lambda part, buf, k: drivers.differ(part, una, core_check, shard(k), CHECK_TIMEOUT, buf,
                                            z3=z3),
        rows, WORKERS)
    unknown_seen = {}
    for r in differ_rows:
        if r["verdict"] not in ("accepted", "rejected"):
            errors.append(f"differ {r['name']}: {r['verdict']} {str(r.get('detail', ''))[:200]}")
        if r.get("finding"):
            findings.append(f"differ {r['name']}: {r.get('differ_detail') or 'the re-check rejects'}")
        if r.get("differ") == "unknown":
            unknown_seen[r["name"]] = r.get("differ_detail") or ""
        if r["class"] == RESIDUAL and r["verdict"] == "accepted":
            if r.get("invalid"):
                errors.append(f"differ {r['name']}: residual row invalid ({r.get('full_kind')})")
            if r.get("triage"):
                findings.append(f"differ {r['name']}: the full check accepts a public print after "
                                "the step")
            if r.get("triage_tier0"):
                notes.append(f"differ {r['name']}: tier 0 accepts a public print after the step, "
                             "and the full check decides it")
    for name in sorted(set(unknown_seen) - set(PINNED_UNKNOWN)):
        errors.append(f"differ {name}: an unknown that is not pinned: {unknown_seen[name]}")
    for name in sorted(set(PINNED_UNKNOWN) - set(unknown_seen)):
        errors.append(f"differ {name}: a pinned unknown is no longer unknown")
    for name in sorted(set(PINNED_UNKNOWN) & set(unknown_seen)):
        if unknown_seen[name] != PINNED_UNKNOWN[name]:
            errors.append(f"differ {name}: unknown for another reason: {unknown_seen[name]}")
    differ_clean = sum(1 for r in differ_rows if r.get("differ") == "clean")
    differ_residual = sum(1 for r in differ_rows if r.get("differ") == "residual")
    if differ_clean < MIN_DIFFER_CLEAN:
        errors.append(f"differ: {differ_clean} programs read clean, under the floor of "
                      f"{MIN_DIFFER_CLEAN}")
    if differ_residual != PINNED_RESIDUAL:
        errors.append(f"differ: {differ_residual} residual programs compared, and the gate pins "
                      f"{PINNED_RESIDUAL}")

    # Canaries.
    canaries = {}
    canaries["differ"] = differ_canary(core_check, work)
    crej = canary_reject()
    buf = io.StringIO()
    drivers.must_reject([dict(crej.manifest(), digest=crej.digest())], una, work, CHECK_TIMEOUT,
                        buf, build=lambda row: crej)
    canaries["reject"] = bool(json.loads(buf.getvalue()).get("finding"))
    canary_rows = {
        "oracle": oracle_canary(canary_oracle(), una, work),
        "internal": oracle_canary(canary_internal(), una, work, planted("internal")),
        "redacted": oracle_canary(canary_redacted(), una, work, planted("public")),
    }
    for name, row in canary_rows.items():
        canaries[name] = caught(name, row)
    with open(os.path.join(root, "canaries.jsonl"), "w", encoding="utf-8") as f:
        f.write(json.dumps({"caught": canaries, "rows": canary_rows}, sort_keys=True) + "\n")

    for name, lines in (("reject", reject_rows), ("keep", keep_rows), ("oracle", oracle_rows),
                        ("differ", differ_rows)):
        with open(os.path.join(root, f"{name}.jsonl"), "w", encoding="utf-8") as f:
            for line in lines:
                f.write(json.dumps(line, sort_keys=True) + "\n")

    sizes = {
        "programs": len(progs),
        "reject": {"expected": len(judged), "judged": len(reject_rows)},
        "keep": {"expected": expected_keep(progs), "judged": len(keep_rows)},
        "oracle": {"expected": len(varied), "judged": len(oracle_rows),
                   "set_aside": sum(1 for r in oracle_rows if r.get("set_aside")),
                   "programs_compared": programs_compared, "variants_compared": compared,
                   "live": live},
        "differ": {"expected": len(rows), "judged": len(differ_rows), "clean": differ_clean,
                   "unknown": len(unknown_seen), "residual": differ_residual},
    }
    for key in ("reject", "keep", "oracle", "differ"):
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
    if live < MIN_LIVE:
        errors.append(f"oracle: {live} programs whose variants differ at the literal's level, "
                      f"under the floor of {MIN_LIVE}")
    if programs_compared < MIN_ORACLE_COMPARED:
        errors.append(f"oracle: {programs_compared} programs compared, under the floor of "
                      f"{MIN_ORACLE_COMPARED}")
    for key, want in PINNED.items():
        got = sizes[key] if key == "programs" else sizes[key]["expected"]
        if got != want:
            errors.append(f"{key}: the fixed run has {got} cases, and the gate pins {want}")
    for name, seen in sorted(canaries.items()):
        if not seen:
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
            "differ": {" ".join(map(str, k)): n for k, n in sorted(differ_tally.items())},
        },
    }
    with open(os.path.join(root, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=1, sort_keys=True)
        f.write("\n")
    print(f"hunt gate: seed {SEED}, {len(progs)} programs, una tip {tip}")
    for key in ("reject", "keep", "oracle", "differ"):
        print(f"  {key}: {sizes[key]['judged']} of {sizes[key]['expected']} cases judged")
    print(f"  oracle: {programs_compared} programs and {compared} variants compared (complete "
          f"against complete), {live} live, {sizes['oracle']['set_aside']} programs set aside")
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
