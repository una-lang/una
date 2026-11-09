"""The hunt's command line.

    python3 -m hunt gen --ops T1,T4 --count 50 --seed 7 --out DIR
    python3 -m hunt reject --manifest DIR/manifest.jsonl --una PATH --root DIR
    python3 -m hunt keep --manifest DIR/manifest.jsonl --una PATH --root DIR
    python3 -m hunt repro --op T4 --seed SEED --edition 0.2 --out DIR
    python3 -m hunt gate --una PATH --out DIR --tip SHA
    python3 -B -m hunt timing --una PATH --out DIR [--seed 973] [--count 40] [--z3 PATH]

`gen --ops W` and `repro --op W1` reach family W (tasks and time, #973), which no default
selection and no gate includes: `timing` runs it, through must-reject and the oracle under the
cost clock (`spec/27 §8` rule 6, `hunt/costclock.py`).

`gen` and `repro` only write files. `reject`, `keep`, `gate` and `timing` run the compiler,
which is heavy work: only in a machine slot. `gate` is the release gate's fixed run
(`hunt/gate.py`).
"""

import argparse
import os
import sys
from collections import Counter

from . import costclock, drivers, gate
from .families import OPERATORS, TIMING_OPERATORS, family_of
from .families.timing import EDITIONS as TIMING_EDITIONS
from .program import derive_seed, write_manifest


def select_ops(spec):
    if not spec:
        return sorted(OPERATORS, key=lambda o: (family_of(o), int(o[len(family_of(o)):])))
    out = []
    every = {**OPERATORS, **TIMING_OPERATORS}
    for part in spec.split(","):
        part = part.strip()
        if part in every:
            out.append(part)
        else:
            fam = [o for o in every if family_of(o) == part]
            if not fam:
                raise SystemExit(f"unknown operator or family {part}")
            out += sorted(fam, key=lambda o: int(o[len(part):]))
    return out


def cmd_gen(a):
    programs = []
    for op in select_ops(a.ops):
        for i in range(a.count):
            seed = derive_seed(a.seed, op, i)
            # Family W has the constructs of edition 0.4 alone.
            editions = TIMING_EDITIONS if op in TIMING_OPERATORS else a.editions.split(",")
            for edition in editions:
                programs.append(drivers._build(op, seed, edition))
    root = drivers.campaign_root(a.out)
    for p in programs:
        p.write(root)
    write_manifest(f"{root}/manifest.jsonl", programs)
    tally = Counter((p.op, p.cls) for p in programs)
    for (op, cls), n in sorted(tally.items()):
        print(f"{op}\t{cls}\t{n}")
    print(f"{len(programs)} programs in {root}")


def cmd_repro(a):
    p = drivers._build(a.op, a.seed, a.edition)
    path = p.write(drivers.campaign_root(a.out))
    print(path)
    print(p.digest())


def cmd_run(a, which):
    rows = drivers.read_manifest(a.manifest)
    root = drivers.campaign_root(a.root)
    out_path = a.results or f"{root}/{which}.jsonl"
    with open(out_path, "w", encoding="utf-8") as out:
        if which == "reject":
            tally = drivers.must_reject(rows, a.una, root, a.timeout, out)
        elif which == "differ":
            tally = drivers.differ(rows, a.una, os.path.realpath(a.core_check), root,
                                   a.timeout, out,
                                   z3=os.path.realpath(a.z3) if a.z3 else None)
        else:
            tally = drivers.must_keep(rows, a.una, root, a.timeout, out,
                                      tuple(a.refactors.split(",")))
    for key, n in sorted(tally.items()):
        print("\t".join(map(str, key)) + f"\t{n}")
    print(out_path)


def main(argv=None):
    ap = argparse.ArgumentParser(prog="hunt")
    sub = ap.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("gen")
    g.add_argument("--ops", default="")
    g.add_argument("--count", type=int, default=10)
    g.add_argument("--seed", type=int, required=True)
    g.add_argument("--editions", default="0.1,0.2")
    g.add_argument("--out", required=True)
    r = sub.add_parser("repro")
    r.add_argument("--op", required=True)
    r.add_argument("--seed", type=int, required=True)
    r.add_argument("--edition", required=True)
    r.add_argument("--out", required=True)
    for name in ("reject", "keep", "differ"):
        c = sub.add_parser(name)
        c.add_argument("--manifest", required=True)
        c.add_argument("--una", required=True)
        c.add_argument("--root", required=True)
        c.add_argument("--timeout", type=float, default=60)
        c.add_argument("--results")
        if name == "keep":
            c.add_argument("--refactors", default="R6,R9")
        if name == "differ":
            c.add_argument("--core-check", required=True,
                           help="the una-core-check executable (formal/.lake/build/bin)")
            c.add_argument("--z3", help="the solver for the residual class's full check "
                           "(#548); without it those rows are invalid")
    gt = sub.add_parser("gate")
    gt.add_argument("--una", required=True)
    gt.add_argument("--out", required=True)
    gt.add_argument("--tip", required=True)
    gt.add_argument("--core-check", required=True,
                    help="the una-core-check executable (formal/.lake/build/bin)")
    gt.add_argument("--z3", required=True, help="the pinned solver, for the residual rows")
    tm = sub.add_parser("timing")
    tm.add_argument("--una", required=True)
    tm.add_argument("--out", required=True)
    tm.add_argument("--seed", type=int, default=costclock.SEED)
    tm.add_argument("--count", type=int, default=costclock.COUNT,
                    help="programs per operator of family W")
    tm.add_argument("--z3", help="the pinned solver: its directory goes first on the runs' PATH")
    a = ap.parse_args(argv)
    if a.cmd == "gate":
        return gate.run(os.path.realpath(a.una), a.out, a.tip, os.path.realpath(a.core_check),
                        os.path.realpath(a.z3))
    if a.cmd == "timing":
        return costclock.run(os.path.realpath(a.una), a.out, a.seed, a.count,
                             os.path.realpath(a.z3) if a.z3 else None)
    if a.cmd == "gen":
        cmd_gen(a)
    elif a.cmd == "repro":
        cmd_repro(a)
    else:
        cmd_run(a, a.cmd)


if __name__ == "__main__":
    sys.exit(main())
