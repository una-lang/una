"""Runs each case (a module `m` with `main`) in I, I-json, N (release) and D (debug runtime).

Usage: UNA_TREE=<una source tree, built with `cargo build --release -p una-cli -p una-rt` and
`cargo build -p una-rt`> Z3_DIR=<Z3 5.1.0 bin directory> python3 run.py sem.py [case ...]
"""

import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

D = Path(__file__).resolve().parent
V = Path(os.environ["UNA_TREE"])
UNA = str(V / "target/release/una")
DEBUG_RT = str(V / "target/debug/libuna_rt.a")
ENV = dict(os.environ)
ENV["PATH"] = os.environ["Z3_DIR"] + ":" + ENV["PATH"]
LOCK = (V / "verification.lock").read_text()


def pkg(name: str, src: str) -> Path:
    d = D / "work" / name
    if d.exists():
        shutil.rmtree(d)
    (d / "src").mkdir(parents=True)
    (d / "una.toml").write_text('[package]\nname = "c"\nversion = "0.1.0"\nedition = "0.2"\n\n[run]\nentry = "m"\ngrants = ["console"]\n')
    (d / "verification.lock").write_text(LOCK)
    (d / "src/m.una").write_text(src)
    return d


def sh(cmd, cwd, env=ENV, timeout=900):
    t = time.perf_counter()
    try:
        p = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr, time.perf_counter() - t
    except subprocess.TimeoutExpired:
        return "timeout", "", "", timeout


def one(name: str, src: str, expected: str | None = None):
    d = pkg(name, src)
    sh([UNA, "fmt", "."], d)
    rec = {"name": name}
    c, o, e, _ = sh([UNA, "--json", "check", "."], d)
    diags = [x for x in json.loads(o)["diagnostics"] if x["severity"] == "error"] if o.strip().startswith("{") else [{"code": "?", "message": o + e}]
    if diags:
        rec["check"] = [(x["code"], x.get("primary", {}).get("start"), x["message"][:160]) for x in diags]
        return rec
    rec["I"] = sh([UNA, "run", "."], d)[:3]
    c, o, e, _ = sh([UNA, "--json", "run", "."], d)
    try:
        rec["I_trace"] = json.loads(o)["trace"]
    except Exception:
        rec["I_trace"] = None
    c, o, e, _ = sh([UNA, "build", ".", "--out", str(d / "n.bin")], d)
    if c != 0:
        rec["N_build"] = (c, (o + e)[-800:])
        return rec
    rec["N"] = sh([str(d / "n.bin"), "--trace", str(d / "n.trace")], d)[:3]
    rec["N_trace"] = json.loads((d / "n.trace").read_text()) if (d / "n.trace").exists() else None
    env = dict(ENV, UNA_RT_LIB=DEBUG_RT)
    c, o, e, _ = sh([UNA, "build", ".", "--out", str(d / "d.bin")], d, env=env)
    if c != 0:
        rec["D_build"] = (c, (o + e)[-800:])
        return rec
    rec["D"] = sh([str(d / "d.bin")], d)[:3]
    m = re.search(r"copied: (\d+)", rec["D"][2])
    rec["copies"] = int(m.group(1)) if m else 0
    probs = []
    if rec["I"] != rec["N"]:
        probs.append(f"I/N differ: I={rec['I']} N={rec['N']}")
    if rec["I_trace"] != rec["N_trace"]:
        probs.append("trace differs")
    if rec["D"][0] != rec["N"][0] or rec["D"][1] != rec["N"][1]:
        probs.append(f"D differs: {rec['D']}")
    if "internal error" in rec["D"][2]:
        probs.append("D internal error: " + rec["D"][2][-300:])
    if expected is not None and rec["I"][1] != expected:
        probs.append(f"SEMANTICS: expected {expected!r}, I printed {rec['I'][1]!r}")
    if expected is not None and rec["N"][1] != expected:
        probs.append(f"SEMANTICS: expected {expected!r}, N printed {rec['N'][1]!r}")
    rec["problems"] = probs
    return rec


if __name__ == "__main__":
    import importlib.util
    spec = importlib.util.spec_from_file_location("cases", sys.argv[1])
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    only = set(sys.argv[2:])
    out = []
    for name, src, expected in mod.CASES:
        if only and name not in only:
            continue
        r = one(name, src, expected)
        out.append(r)
        if "check" in r:
            print(f"{name:34} CHECK FAILED {r['check']}", flush=True)
        elif "N_build" in r or "D_build" in r:
            print(f"{name:34} BUILD FAILED {r.get('N_build') or r.get('D_build')}", flush=True)
        else:
            print(f"{name:34} copies={r['copies']:<4} " + ("OK" if not r["problems"] else "PROBLEMS " + " | ".join(r["problems"])), flush=True)
    (D / f"results-{Path(sys.argv[1]).stem}.json").write_text(json.dumps(out, indent=1))
