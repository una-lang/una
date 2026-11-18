"""Area 5 of #308: parse_json then to_json on inputs that double in size, timed in both
engines. Usage: python3 scale.py <una binary>"""
import json, os, random, subprocess, sys, time
HERE = os.path.dirname(os.path.abspath(__file__))
PKG = os.path.join(HERE, "scale")
una = os.path.abspath(sys.argv[1])
subprocess.run([una, "build", "."], cwd=PKG, check=True, capture_output=True)
rng = random.Random(5)

def flat(n):  # an array of numbers, strings and decimals
    return "[" + ",".join(rng.choice(['12345', '"abcdé😀"', '1.25e3', 'null', 'true']) for _ in range(n)) + "]"

def wide(n):  # one object with n keys in random order
    keys = [f'"k{rng.randrange(10**9)}_{i}"' for i in range(n)]
    return "{" + ",".join(f"{k}:{i}" for i, k in enumerate(keys)) + "}"

def deep(n):  # n arrays each nested 500 deep
    return "[" + ",".join("[" * 500 + "1" + "]" * 500 for _ in range(n)) + "]"

def escaped(n):  # strings full of escapes
    return "[" + ",".join('"\\u00e9\\n\\ud83d\\ude00\\"x"' for _ in range(n)) + "]"

def run(cmd):
    t = time.perf_counter(); r = subprocess.run(cmd, cwd=PKG, capture_output=True, text=True); dt = time.perf_counter() - t
    if r.returncode: sys.exit(r.stdout + r.stderr)
    return dt

import itertools
SIZES = [("flat", flat, 200000), ("wide", wide, 50000), ("deep", deep, 200), ("escaped", escaped, 100000)]
for name, f, base in SIZES:
    rows = []
    for k in range(4):
        n = base * 2 ** k
        text = f(n)
        open(os.path.join(PKG, "io", "in.json"), "w").write(text)
        ti = min(run([una, "run", "."]) for _ in range(3)); tn = min(run(["./target/una/scale"]) for _ in range(3))
        out = open(os.path.join(PKG, "io", "out.json")).read()
        rows.append((n, len(text), ti, tn, out[:20]))
    print(name)
    prev = None
    for n, size, ti, tn, head in rows:
        ratio = "" if prev is None else f"  x{ti/prev[0]:.2f} interp, x{tn/prev[1]:.2f} native"
        print(f"  n={n:>8} {size/1e6:7.2f} MB  interp {ti:6.2f}s  native {tn:6.2f}s{ratio}  {head!r}")
        prev = (ti, tn)
