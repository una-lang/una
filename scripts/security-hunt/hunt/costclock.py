"""The hunt's timing run (#973, ADR 0117 S8): family W through must-reject and the two-run
oracle under the cost clock (`oracle.py`, `spec/27 §8` rule 6).

    python3 -B -m hunt timing --una PATH --out DIR [--seed 973] [--count 40] [--z3 PATH]

It is not the release gate's fixed run (`gate.py`, seed 767), and it never changes it: family W is
in `TIMING_OPERATORS`, apart from the operators the gate runs, and has a seed of its own. It runs
at release validation or overnight, as heavy work (`docs/release-gates.md`, "The hunt's timing
run"). It fails closed: `summary.json` says PASS only when there is no finding and no error, the
canaries behave, every operator with an accepted form compared at least one program, and W1 and
W2 each have a live program (`coverage_errors`).

The canaries:
- `canary-cost`: an accepted program whose `within` runs a loop on a public literal that its
  manifest marks as secret, so the oracle varies it. Under the cost clock the loop's length
  decides the time-out, which is printed: the oracle must call it a leak. A cost clock that does
  not move, or a comparison that misses the value, fails the run.
- `canary-plain`: the same program on the plain virtual clock, where work takes no time: it must
  be clean, so the leak of the first comes from the cost clock alone.
"""

import atexit
import io
import json
import os
import time
from collections import Counter

from . import drivers, native, oracle, sandbox
from .families import TIMING_OPERATORS, family_of
from .gate import _sharded
from .itaty import lock_text
from .packages import package_files
from .program import OPEN, Program, derive_seed, write_manifest
from .una import Varied, header, join_decls

SEED = 973
# Programs per operator: the smallest count, of 25, 30 and 40, at which every form of family W is
# in the run (a unit test checks it).
COUNT = 40
EDITION = "0.4"
CHECK_TIMEOUT = 60
RUN_TIMEOUT = 20
WORKERS = 4


def programs(seed=SEED, count=COUNT):
    """Every program of the timing run, in a fixed order: `count` seeds of each operator of
    `TIMING_OPERATORS`, edition 0.4."""
    return [TIMING_OPERATORS[op](derive_seed(seed, op, i), EDITION)
            for op in sorted(TIMING_OPERATORS) for i in range(count)]


def canary_cost():
    """The cost clock's canary: a public loop's length, which the manifest marks as secret,
    decides whether a `within` times out, and the outcome is printed."""
    v = Varied()
    lit = v.lit("Int", "secret", "30")
    text = join_decls(
        header("main", imports=[("std.time", ["millisecond"])]),
        "tally(n: Int) -> Int\n=\n  let m = if n > 200 then 200 else n\n  let edit acc = 0\n"
        "  for i in 0..<m\n    acc = acc + 1\n  acc",
        "pub main() -> Unit\n  uses @console: Console[print]\n=\n"
        f"  let n: Int = {lit}\n  let r = tally(n) within 50 * millisecond\n"
        '    if error use -1\n  let late = r == -1\n  @console.print("{late}")')
    files = package_files("canary", EDITION, {"main": text}, entry="main", grants=["console"])
    files["ita.lock"] = lock_text()
    files, vary = v.place(files)
    return Program(op="canary-cost", rule="the timing run's own canary", edition=EDITION, seed=0,
                   cls=OPEN, files=files, vary=vary)


def canary(prog, una, work, cost):
    """Runs one program through the oracle, with or without the cost clock; its result row."""
    buf = io.StringIO()
    oracle.oracle([dict(prog.manifest(), digest=prog.digest())], una, work, RUN_TIMEOUT, buf,
                  lambda row: prog, cost=cost)
    return json.loads(buf.getvalue())


def canary_errors(cost_row, plain_row):
    """The canaries' verdicts: the cost clock's must be a clean leak, the plain clock's clean."""
    errors = []
    if cost_row.get("error") or not cost_row.get("finding") or cost_row.get("compared", 0) < 1:
        errors.append("canary-cost: under the cost clock, a deadline that a varied loop decides "
                      f"is not seen (error {cost_row.get('error')!r}, finding "
                      f"{cost_row.get('finding')}, compared {cost_row.get('compared')})")
    if plain_row.get("error") or plain_row.get("finding") or plain_row.get("compared", 0) < 1:
        errors.append("canary-plain: on the plain virtual clock the same program is not clean "
                      f"(error {plain_row.get('error')!r}, finding {plain_row.get('finding')}, "
                      f"compared {plain_row.get('compared')})")
    return errors


# The operators whose secret decides a deadline in some accepted form: each must have a live
# program, one whose variants differ at the secret's own level (a `within` timed out for some
# values and not for others), or the cost clock did not reach the constructs it is for.
LIVE_OPERATORS = ("W1", "W2")


def coverage_errors(progs, oracle_rows):
    """An operator with an accepted form that compared no program fails the run, and so does one
    of `LIVE_OPERATORS` with no live program."""
    accepted = {p.op for p in progs if not p.expect}
    compared = Counter(r["op"] for r in oracle_rows if r.get("compared", 0) > 0)
    live = Counter(r["op"] for r in oracle_rows if r.get("live"))
    errors = [f"oracle: operator {op} has accepted forms, and compared no program"
              for op in sorted(accepted) if compared[op] == 0]
    errors += [f"oracle: operator {op} has no live program: no secret moved a deadline"
               for op in LIVE_OPERATORS if op in accepted and live[op] == 0]
    return errors


def run(una, out_dir, seed=SEED, count=COUNT, z3=None):
    """Runs the timing run. Writes OUT/manifest.jsonl, reject.jsonl, oracle.jsonl, canaries.jsonl
    and summary.json, prints the summary, and returns 0 (PASS) or 1 (FAIL)."""
    if os.path.exists(out_dir) and os.listdir(out_dir):
        raise SystemExit(f"hunt timing: {out_dir} is not empty; the run writes a fresh directory")
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
    progs = programs(seed, count)
    if len({p.name for p in progs}) != len(progs):
        raise SystemExit("two programs of the timing run share a name")
    write_manifest(os.path.join(root, "manifest.jsonl"), progs)
    rows = drivers.read_manifest(os.path.join(root, "manifest.jsonl"))
    work = os.path.join(root, "work")
    os.makedirs(work, exist_ok=True)

    def shard(k):
        path = os.path.join(work, f"part{k}")
        os.makedirs(path, exist_ok=True)
        return path
    errors, findings = [], []

    reject_tally, reject_rows = _sharded(
        lambda part, buf, k: drivers.must_reject(part, una, shard(k), CHECK_TIMEOUT, buf), rows,
        WORKERS)
    for r in reject_rows:
        if r["verdict"] not in ("accepted", "rejected"):
            errors.append(f"reject {r['name']}: {r['verdict']} {r.get('detail', '')[:200]}")
        if r.get("finding"):
            findings.append(f"reject {r['name']}: a closed call, which a correct checker "
                            "rejects, is accepted")

    oracle_tally, oracle_rows = _sharded(
        lambda part, buf, k: oracle.oracle(part, una, shard(k), RUN_TIMEOUT, buf,
                                           drivers.rebuild, cost=True),
        rows, WORKERS)
    for r in oracle_rows:
        if r.get("error"):
            errors.append(f"oracle {r['name']}: {r['error'][:300]}")
        if r.get("finding"):
            findings.append(f"oracle {r['name']}: what an observer below the secret sees differs "
                            "under the cost clock")
    errors.extend(coverage_errors(progs, oracle_rows))

    cost_row = canary(canary_cost(), una, shard("canary-cost"), True)
    plain_row = canary(canary_cost(), una, shard("canary-plain"), False)
    errors.extend(canary_errors(cost_row, plain_row))

    for name, lines in (("reject.jsonl", reject_rows), ("oracle.jsonl", oracle_rows),
                        ("canaries.jsonl", [cost_row, plain_row])):
        with open(os.path.join(root, name), "w", encoding="utf-8") as f:
            for line in lines:
                f.write(json.dumps(line, sort_keys=True) + "\n")
    families = Counter(family_of(p.op) for p in progs)
    summary = {
        "verdict": "FAIL" if errors or findings else "PASS",
        "seed": seed,
        "count": count,
        "programs": len(progs),
        "families": dict(families),
        "reject": {"/".join(map(str, k)): n for k, n in sorted(reject_tally.items())},
        "oracle": dict(sorted(oracle_tally.items())),
        "compared": sum(r.get("compared", 0) for r in oracle_rows),
        "live": dict(sorted(Counter(r["op"] for r in oracle_rows if r.get("live")).items())),
        "findings": findings,
        "errors": errors,
        "seconds": round(time.monotonic() - started, 1),
    }
    with open(os.path.join(root, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2, sort_keys=True)
        f.write("\n")
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if summary["verdict"] == "PASS" else 1
