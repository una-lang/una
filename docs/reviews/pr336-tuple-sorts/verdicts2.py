"""(a) cold run per binary with every .una dir removed first; (b) stale-hit test with one shared key."""
import json, os, shutil, subprocess, tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
S = Path(__file__).resolve().parent.parent
BINS = {"81cf9a8": S / "una-81cf9a8/target/release/una", "336": S / "una-336/target/release/una"}
progs = Path("programs.txt").read_text().split()
def wipe():
    for root in (S / "una-336/tests", S / "una-336/examples", S / "r336/cases"):
        for d in list(root.rglob(".una")):
            if d.is_dir() and S in d.parents:
                shutil.rmtree(d)
def run(una, home, p):
    env = dict(os.environ, HOME=home, PATH=os.environ["Z3_DIR"] + ":" + os.environ["PATH"])
    path = Path(p); cwd, arg = (path, ".") if path.is_dir() else (path.parent, path.name)
    r = subprocess.run([str(una), "--json", "obligations", arg], cwd=cwd, env=env, capture_output=True, text=True, timeout=900)
    try:
        return p, (r.returncode, sorted((o["id"], o["status"], o.get("cached")) for o in json.loads(r.stdout).get("obligations", [])))
    except Exception:
        return p, (r.returncode, "NONJSON")
def sweep(una, home):
    with ThreadPoolExecutor(8) as ex:
        return dict(ex.map(lambda p: run(una, home, p), progs))
strip = lambda r: {p: (c, [(i, s) for i, s, _ in o] if isinstance(o, list) else o) for p, (c, o) in r.items()}
cached = lambda r: sum(1 for c, o in r.values() if isinstance(o, list) for _, _, h in o if h)
# (a) cold
cold = {}
for k, una in BINS.items():
    wipe(); home = tempfile.mkdtemp(prefix="home-", dir=".")
    cold[k] = sweep(una, home); shutil.rmtree(home)
    print(f"(a) {k}: cached hits {cached(cold[k])}")
a, b = strip(cold["81cf9a8"]), strip(cold["336"])
print("(a) cold verdicts differ in", sum(1 for p in progs if a[p] != b[p]), "programs of", len(progs))
# (b) shared key: warm with old, then new
wipe(); home = tempfile.mkdtemp(prefix="home-", dir=".")
warm = sweep(BINS["81cf9a8"], home)
after = sweep(BINS["336"], home)
tuple_ids = set()
for d in Path("q-336").iterdir():
    prog = (d / "program.txt").read_text()
    if any("i!Tuple" in q.read_text() for q in d.glob("*.smt2")):
        tuple_ids.add(prog)
hits_tuple = sum(1 for p in tuple_ids if p in after and isinstance(after[p][1], list) for _, _, h in after[p][1] if h)
hits_other = sum(1 for p in progs if p not in tuple_ids and isinstance(after[p][1], list) for _, _, h in after[p][1] if h)
print(f"(b) after warming with 81cf9a8: 169b880 cache hits in tuple programs {hits_tuple} (obligations of {len(tuple_ids)} programs), hits elsewhere {hits_other}")
print("(b) verdicts equal to cold:", strip(after) == b)
second = sweep(BINS["336"], home)
print(f"(b) third run (169b880 again): cached hits {cached(second)}, verdicts equal: {strip(second) == b}")
shutil.rmtree(home); wipe()
