"""Scaling: each shape at 3 sizes; interpreter and release native time (best of 3), debug copy count."""

import json
import re
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run as R  # noqa: E402

HDR = "module m\n\npub kind Limit = | Full\n\n"
MAIN = "pub main() -> Unit\n  uses @console: Console[print]\n=\n"

SHAPES = {
    "direct": ("", "  let edit acc: List[Int] = []\n  for i in 0..<N\n    acc = acc.append(item: i)\n  @console.print(\"{acc.len}\")\n"),
    "edit1": ("p1(edit acc: List[Int], i: Int) -> Unit\n=\n  acc = acc.append(item: i)\n\n",
              "  let edit acc: List[Int] = []\n  for i in 0..<N\n    p1(edit acc, i: i)\n  @console.print(\"{acc.len}\")\n"),
    "edit3": ("p1(edit acc: List[Int], i: Int) -> Unit\n=\n  acc = acc.append(item: i)\n\np2(edit acc: List[Int], i: Int) -> Unit\n=\n  p1(edit acc, i: i)\n\np3(edit acc: List[Int], i: Int) -> Unit\n=\n  p2(edit acc, i: i)\n\n",
              "  let edit acc: List[Int] = []\n  for i in 0..<N\n    p3(edit acc, i: i)\n  @console.print(\"{acc.len}\")\n"),
    "ret1": ("r1(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)\n\n",
             "  let edit acc: List[Int] = []\n  for i in 0..<N\n    acc = r1(acc, i: i)\n  @console.print(\"{acc.len}\")\n"),
    "ret3": ("r1(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)\n\nr2(acc: List[Int], i: Int) -> List[Int] = r1(acc, i: i)\n\nr3(acc: List[Int], i: Int) -> List[Int]\n=\n  let x = r2(acc, i: i)\n  x\n\n",
             "  let edit acc: List[Int] = []\n  for i in 0..<N\n    acc = r3(acc, i: i)\n  @console.print(\"{acc.len}\")\n"),
    "text-edit2": ("t1(edit out: Text, i: Int) -> Unit\n=\n  out = \"{out}x\"\n\nt2(edit out: Text, i: Int) -> Unit\n=\n  t1(edit out, i: i)\n\n",
                   "  let edit s = \"\"\n  for i in 0..<N\n    t2(edit s, i: i)\n  @console.print(\"{s.len}\")\n"),
    "text-ret": ("tr(out: Text) -> Text = \"{out}x\"\n\n",
                 "  let edit s = \"\"\n  for i in 0..<N\n    s = tr(s)\n  @console.print(\"{s.len}\")\n"),
    "generic": ("grow[T](xs: List[T], x: T) -> List[T] = xs.append(item: x)\n\n",
                "  let edit acc: List[Text] = []\n  for i in 0..<N\n    acc = grow(acc, x: \"v\")\n  @console.print(\"{acc.len}\")\n"),
    "fn-value": ("r1(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)\n\nrun_with(f: fn(List[Int], Int) -> List[Int], n: Int) -> Int\n=\n  let edit acc: List[Int] = []\n  for i in 0..<n\n    acc = f(acc, i)\n  acc.len\n\n",
                 "  @console.print(\"{run_with(r1, n: N)}\")\n"),
    "recursion": ("build(acc: List[Int], n: Int) -> List[Int] = if n == 0 then acc else build(acc.append(item: n), n: n - 1)\n\n",
                  "  @console.print(\"{build([], n: N).len}\")\n"),
    "tasks": ("r1(acc: List[Int], i: Int) -> List[Int] = acc.append(item: i)\n\n",
              "  let lens = together for k in 0..<4\n    let edit acc: List[Int] = []\n    for i in 0..<N\n      acc = r1(acc, i: i)\n    acc.len\n  @console.print(\"{lens.len}\")\n"),
    "map-edit": ("put(edit m: Map[Int, Int], k: Int) -> Unit\n=\n  edit m.set(key: k, value: k)\n\n",
                 "  let edit m: Map[Int, Int] = empty_map()\n  for i in 0..<N\n    put(edit m, k: i)\n  @console.print(\"{m.len}\")\n"),
    "keep-on-error": ("bounded(acc: List[Int], i: Int) -> List[Int]\n  fails Limit\n=\n  if i < 0\n    fail Full\n  acc.append(item: i)\n\n",
                      "  let edit acc: List[Int] = []\n  for i in 0..<N\n    acc = bounded(acc, i: i)\n      if error use acc\n  @console.print(\"{acc.len}\")\n"),
    "keep-on-error-pass": ("bounded(acc: List[Int], i: Int) -> List[Int]\n  fails Limit\n=\n  if i < 0\n    fail Full\n  acc.append(item: i)\n\nrun_all(n: Int) -> Int\n  fails Limit\n=\n  let edit acc: List[Int] = []\n  for i in 0..<n\n    acc = bounded(acc, i: i)\n      if error pass\n  acc.len\n\n",
                           "  let n = run_all(N)\n    if error use 0\n  @console.print(\"{n}\")\n"),
    "fold-append": ("", "  let edit src: List[Int] = []\n  for i in 0..<N\n    src = src.append(item: i)\n  let out = src.fold(initial: [], step: fn(acc, x) -> acc.append(item: x))\n  @console.print(\"{out.len}\")\n"),
    "fold-text": ("", "  let edit src: List[Int] = []\n  for i in 0..<N\n    src = src.append(item: 1)\n  let out = src.fold(initial: \"\", step: fn(line, x) -> \"{line}{x}\")\n  @console.print(\"{out.len}\")\n"),
    "lambda-local": ("", "  let add = fn(acc: List[Int], i: Int) -> acc.append(item: i)\n  let edit acc: List[Int] = []\n  for i in 0..<N\n    acc = add(acc, i)\n  @console.print(\"{acc.len}\")\n"),
    "nested-loops": ("", "  let edit acc: List[Int] = []\n  let edit t = \"\"\n  for i in 0..<100\n    for j in 0..<(N / 100)\n      acc = acc.append(item: j)\n      t = \"{t}y\"\n  @console.print(\"{acc.len} {t.len}\")\n"),
    "task-text": ("", "  let lens = together for k in 0..<4\n    let edit t = \"\"\n    for i in 0..<N\n      t = \"{t}z\"\n    t.len\n  @console.print(\"{lens.len}\")\n"),
    "task-nested": ("", "  let lens = together for k in 0..<2\n    let edit acc: List[Int] = []\n    for i in 0..<10\n      for j in 0..<(N / 10)\n        acc = acc.append(item: j)\n    acc.len\n  @console.print(\"{lens.len}\")\n"),
}


def timed(cmd, cwd, env=None):
    best = None
    R.sh(cmd, cwd, env=env or R.ENV)
    for _ in range(3):
        c, o, e, t = R.sh(cmd, cwd, env=env or R.ENV)
        best = t if best is None else min(best, t)
    return c, o.strip(), round(best, 3)


def measure(shape, n):
    lib, body = SHAPES[shape]
    d = R.pkg(f"scale-{shape}-{n}", HDR + lib + MAIN + body.replace("N", str(n)))
    R.sh([R.UNA, "fmt", "."], d)
    c, o, e, _ = R.sh([R.UNA, "build", ".", "--out", str(d / "n.bin")], d)
    if c != 0:
        return {"error": (o + e)[-600:]}
    R.sh([R.UNA, "build", ".", "--out", str(d / "d.bin")], d, env=dict(R.ENV, UNA_RT_LIB=R.DEBUG_RT))
    ci, oi, ti = timed([R.UNA, "run", "."], d)
    cn, on, tn = timed([str(d / "n.bin")], d)
    cd, od, ed, _ = R.sh([str(d / "d.bin")], d)
    m = re.search(r"copied: (\d+)", ed)
    return {"I": (ci, oi, ti), "N": (cn, on, tn), "D": (cd, od.strip(), int(m.group(1)) if m else 0, "internal error" in ed)}


if __name__ == "__main__":
    sizes = [int(x) for x in sys.argv[1].split(",")]
    shapes = sys.argv[2].split(",") if len(sys.argv) > 2 else list(SHAPES)
    out = {}
    for s in shapes:
        for n in sizes:
            r = measure(s, n)
            out[f"{s}-{n}"] = r
            print(s, n, r, flush=True)
    Path(R.D / f"scale-{sys.argv[1].replace(',', '_')}.json").write_text(json.dumps(out, indent=1))
