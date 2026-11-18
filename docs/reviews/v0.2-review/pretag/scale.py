"""Scaling probes: one program per operation and size, timed in `una run` and natively."""
import os, subprocess, sys, time, json
from pathlib import Path
D = Path(__file__).resolve().parent
UNA = sys.argv[1]
ENV = dict(os.environ); ENV["PATH"] = str(Path.home() / "Projects/una-lang/una/target/z3/bin") + ":" + ENV["PATH"]
LOCK = (Path.home() / "Projects/una-lang/una-bench/tasks/A05-loyalty-ledger/una/scaffold/verification.lock").read_text()
SIZES = [int(x) for x in sys.argv[2].split(",")]
BUILD = """pub build(n: Int) -> List[Int]
=
  let edit xs: List[Int] = []
  for i in 0..<n
    xs = xs.append(item: i)
  xs
"""
OPS0 = {
 "append": (BUILD, "  let xs = build(N)\n  @console.print(\"{xs.len}\")"),
 "text_concat": ("""pub grow(n: Int) -> Text
=
  let edit t = ""
  for i in 0..<n
    t = "{t}x"
  t
""", "  let t = grow(N)\n  @console.print(\"{t.len}\")"),
 "at_loop": (BUILD + """
pub total(xs: List[Int]) -> Int
=
  let edit s = 0
  for i in 0..<xs.len
    s = s + xs.at(index: i).or_else(default: 0)
  s
""", "  let xs = build(N)\n  @console.print(\"{total(xs)}\")"),
 "set_loop": (BUILD + """
pub bump(n: Int) -> Int
=
  let edit xs = build(n)
  for i in 0..<n
    if i < xs.len
      set(edit xs, index: i, item: i + 1)
  xs.len
""", "  @console.print(\"{bump(N)}\")"),
 "fold": (BUILD, "  let xs = build(N)\n  let s = xs.fold(initial: 0, step: fn(a, x) -> a + x)\n  @console.print(\"{s}\")"),
}
OPS = {k: v for k, v in OPS0.items() if k in sys.argv[3].split(",")}
res = {}
for op, (lib, body) in OPS.items():
    for n in SIZES:
        d = D / f"{op}-{n}"; (d / "src/p").mkdir(parents=True, exist_ok=True)
        (d / "una.toml").write_text('[package]\nname = "p"\nversion = "0.1.0"\nedition = "0.2"\n\n[run]\nentry = "p.main"\ngrants = ["console"]\n')
        (d / "verification.lock").write_text(LOCK)
        (d / "src/p/main.una").write_text("module p.main\n\n" + lib + "\npub main() -> Unit\n  uses @console: Console[print]\n=\n" + body.replace("N", str(n)) + "\n")
        subprocess.run([UNA, "fmt", "."], cwd=d, env=ENV, capture_output=True)
        b = subprocess.run([UNA, "build", ".", "--out", str(d / "p.bin")], cwd=d, env=ENV, capture_output=True, text=True)
        if b.returncode:
            print(op, n, "BUILD FAILED", b.stdout[-600:], b.stderr[-1200:]); break
        row = {}
        for eng, cmd in (("interp", [UNA, "run", "."]), ("native", [str(d / "p.bin")])):
            subprocess.run(cmd, cwd=d, env=ENV, capture_output=True, timeout=1800)
            best = None
            for _ in range(3):
                a = time.perf_counter(); p = subprocess.run(cmd, cwd=d, env=ENV, capture_output=True, text=True, timeout=1800); el = time.perf_counter() - a
                best = el if best is None else min(best, el)
            row[eng] = (round(best, 3), p.returncode, p.stdout.strip()[:40], p.stderr.strip()[:200])
        res[f"{op}-{n}"] = row
        print(op, n, row, flush=True)
json.dump(res, open(D / f"scale-{sys.argv[3]}.json", "w"), indent=1)
