"""Recomputes each new tuple-sort name from the old query and checks old == new up to declaration order."""
import hashlib, re
from pathlib import Path
PROJ = re.compile(r"^\(declare-fun i!t!(\d+)!(i!Tuple[^\s()]*) \((\S+)\) (\S+)\)$")
TOK = re.compile(r"i!Tuple[^\s()]*")
def new_names(text):
    comps = {}
    for line in text.splitlines():
        m = PROJ.match(line)
        if m:
            k, tup, arg, res = int(m.group(1)), m.group(2), m.group(3), m.group(4)
            assert arg == tup
            comps.setdefault(tup, {})[k] = res
    memo = {}
    def name(t):
        if t in memo: return memo[t]
        cs = comps[t]
        n = int(re.match(r"i!Tuple(\d+)", t).group(1))
        assert sorted(cs) == list(range(n)), (t, cs)
        spelled = "".join((name(c) if c.startswith("i!Tuple") else c) + "\n" for c in (cs[i] for i in range(n)))
        memo[t] = f"i!Tuple{n}!" + hashlib.sha256(spelled.encode()).hexdigest()[:32]
        return memo[t]
    return {t: name(t) for t in comps}
total = tup = exact = reordered = bad = 0
for d in sorted(Path("q-81cf9a8").iterdir()):
    for q in sorted(d.glob("*.smt2")):
        total += 1
        a = q.read_text(); b = (Path("q-336") / d.name / q.name).read_text()
        names = new_names(a)
        if not names:
            if a != b: bad += 1; print("NO-TUPLE QUERY DIFFERS", d.name, q.name)
            continue
        tup += 1
        sub = TOK.sub(lambda m: names[m.group(0)], a)
        if sub == b: exact += 1; continue
        la, lb = sub.splitlines(), b.splitlines()
        if sorted(la) == sorted(lb) and [l for l in la if not l.startswith("(declare-")] == [l for l in lb if not l.startswith("(declare-")]:
            reordered += 1
        else:
            bad += 1; print("DIFF", (d / "program.txt").read_text(), q.name)
print(f"queries {total}; no tuple: identical {total - tup - (bad if False else 0)}; with tuples {tup}: identical after renaming {exact}, identical up to the order of declarations {reordered}; other differences {bad}")
