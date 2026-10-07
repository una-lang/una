"""Per obligation: warm the cache with 81cf9a8, run 169b880 with the same key; no obligation whose query has a tuple sort may hit."""
import json, os, shutil, subprocess, tempfile
from pathlib import Path
S = Path(__file__).resolve().parent.parent
OLD, NEW = S / "una-81cf9a8/target/release/una", S / "una-336/target/release/una"
progs = {}
for qd in ("q-336", "qx-336"):
    for d in Path(qd).iterdir():
        p = (d / "program.txt").read_text()
        qs = sorted(d.glob("*.smt2"))
        if any("i!Tuple" in q.read_text() for q in qs):
            progs[p] = ["i!Tuple" in q.read_text() for q in qs]
home = tempfile.mkdtemp(prefix="home-", dir=".")
env = dict(os.environ, HOME=home, PATH=os.environ["Z3_DIR"] + ":" + os.environ["PATH"])
def obs(una, p):
    path = Path(p); cwd, arg = (path, ".") if path.is_dir() else (path.parent, path.name)
    r = subprocess.run([str(una), "--json", "obligations", arg], cwd=cwd, env=env, capture_output=True, text=True, timeout=900)
    return json.loads(r.stdout)["obligations"]
tot = tup_hit = tup = other_hit = other = mismatch = 0
for p, flags in sorted(progs.items()):
    path = Path(p); cache = (path if path.is_dir() else path.parent) / ".una"
    if cache.is_dir() and S in cache.parents: shutil.rmtree(cache)
    obs(OLD, p)
    o = obs(NEW, p)
    if len(o) != len(flags): mismatch += 1; print("count mismatch", p, len(o), len(flags)); continue
    for ob, has in zip(o, flags):
        if has: tup += 1; tup_hit += ob["cached"]
        else: other += 1; other_hit += ob["cached"]
    if cache.is_dir() and S in cache.parents: shutil.rmtree(cache)
shutil.rmtree(home)
print(f"programs {len(progs)}; obligations with a tuple query {tup}, cache hits among them {tup_hit}; without {other}, hits {other_hit}; mismatches {mismatch}")
