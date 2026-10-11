"""The hunt's command line.

    python3 -m hunt gen --ops T1,T4 --count 50 --seed 7 --out DIR
    python3 -m hunt reject --manifest DIR/manifest.jsonl --una PATH --root DIR
    python3 -m hunt keep --manifest DIR/manifest.jsonl --una PATH --root DIR
    python3 -m hunt repro --op T4 --seed SEED --edition 0.2 --out DIR

`gen` and `repro` only write files. `reject` and `keep` run the compiler, which is heavy
work: only in a machine slot (the coordinator's condition 3).
"""

import argparse
import os
import sys
from collections import Counter

from . import drivers
from .families import OPERATORS, family_of
from .program import derive_seed, write_manifest


def select_ops(spec):
    if not spec:
        return sorted(OPERATORS, key=lambda o: (family_of(o), int(o[len(family_of(o)):])))
    out = []
    for part in spec.split(","):
        part = part.strip()
        if part in OPERATORS:
            out.append(part)
        else:
            fam = [o for o in OPERATORS if family_of(o) == part]
            if not fam:
                raise SystemExit(f"unknown operator or family {part}")
            out += sorted(fam, key=lambda o: int(o[len(part):]))
    return out


def cmd_gen(a):
    programs = []
    for op in select_ops(a.ops):
        for i in range(a.count):
            seed = derive_seed(a.seed, op, i)
            for edition in a.editions.split(","):
                programs.append(OPERATORS[op](seed, edition))
    root = drivers.campaign_root(a.out)
    for p in programs:
        p.write(root)
    write_manifest(f"{root}/manifest.jsonl", programs)
    tally = Counter((p.op, p.cls) for p in programs)
    for (op, cls), n in sorted(tally.items()):
        print(f"{op}\t{cls}\t{n}")
    print(f"{len(programs)} programs in {root}")


def cmd_repro(a):
    p = OPERATORS[a.op](a.seed, a.edition)
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
                                   a.timeout, out)
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
    a = ap.parse_args(argv)
    if a.cmd == "gen":
        cmd_gen(a)
    elif a.cmd == "repro":
        cmd_repro(a)
    else:
        cmd_run(a, a.cmd)


if __name__ == "__main__":
    sys.exit(main())
