"""`una --json obligations` on every program with an empty proof cache per binary; compares ids and statuses."""
import json, os, subprocess, sys, tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
S = Path(__file__).resolve().parent.parent
BINS = {"81cf9a8": S / "una-81cf9a8/target/release/una", "336": S / "una-336/target/release/una"}
progs = Path("programs.txt").read_text().split()
def run(una, home, p):
    env = dict(os.environ, HOME=home, PATH=os.environ["Z3_DIR"] + ":" + os.environ["PATH"])
    env.pop("UNA_PROOF_KEY", None)
    path = Path(p)
    cwd, arg = (path, ".") if path.is_dir() else (path.parent, path.name)
    r = subprocess.run([str(una), "--json", "obligations", arg], cwd=cwd, env=env, capture_output=True, text=True, timeout=900)
    try:
        d = json.loads(r.stdout)
        obs = sorted((o.get("id"), o.get("status")) for o in d.get("obligations", []))
        codes = sorted(x["code"] for x in d.get("diagnostics", []))
    except Exception:
        obs, codes = "NONJSON", r.stdout[:200] + r.stderr[:200]
    return p, (r.returncode, obs, codes)
res = {}
for k, una in BINS.items():
    home = tempfile.mkdtemp(prefix=f"home-{k}-", dir=str(Path.cwd()))
    with ThreadPoolExecutor(8) as ex:
        res[k] = dict(ex.map(lambda p: run(una, home, p), progs))
    print(k, "cache dir:", [str(x.relative_to(home)) for x in Path(home).rglob("*") if x.is_dir()][:3], flush=True)
diff = [p for p in progs if res["81cf9a8"][p] != res["336"][p]]
nob = sum(len(v[1]) for v in res["336"].values() if isinstance(v[1], list))
from collections import Counter
st = Counter(s for v in res["336"].values() if isinstance(v[1], list) for _, s in v[1])
print(f"programs {len(progs)}, obligations {nob}, statuses {dict(st)}, programs that differ {len(diff)}")
for p in diff[:10]: print("DIFF", p, res["81cf9a8"][p], res["336"][p])
json.dump({k: {p: v for p, v in r.items()} for k, r in res.items()}, open("verdicts.json", "w"))
