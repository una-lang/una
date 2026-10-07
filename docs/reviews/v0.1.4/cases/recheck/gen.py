"""Recheck of #343's H-1 fix (edition 0.1): variants of the prelude-HOF class beyond the first
four repros. Each is a runnable package; it leaks when `check` accepts it and `run` prints
different numbers for S="" and S="x". Writes ./cases/<name>/."""
import os, sys
LOCK = sys.argv[1]
HEAD = '''module sw

pub capability Log
  op write(n: Int) -> Unit

pub handler to_console() for Log
  uses @console: Console[print]
  op write(n) = @console.print("{n}")
'''
MAIN = '''
pub main() -> Unit
  uses @console: Console[print]
  uses @env: Env[get]
=
  let s = @env.get("S")
    if none use ""
  run(s.len) with @log = to_console()

run(v: internal Int) -> Unit
  uses @log: Log[write]
=
'''
D = "if a == b then p else q"
cases = {
 # The decision is fold's initial value, not an element.
 "fold_decided_initial": ("g[T](a: T, b: T, xs: List[T], step: fn(Int, T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let n = xs.fold(initial: if a == b then 0 else 1, step: step)\n  @log.write(n)\n",
   "  g([v], b: [0], xs: [[1]], step: fn(acc, ys) -> acc + ys.len)"),
 # The decision is the list's length, not its elements.
 "map_decided_length": ("g[T](a: T, b: T, p: T, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let ns = (if a == b then [p] else [p, p]).map(each: k)\n  @log.write(ns.len)\n",
   "  g([v], b: [0], p: [1], k: fn(ys) -> ys.len)"),
 # The callback returns T, and the result is projected by another callback.
 "map_identity_then_k": (f"g[T](a: T, b: T, p: T, q: T, same: fn(T) -> T, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let ys = [{D}].map(each: same)\n  let ns = ys.map(each: k)\n  @log.write(ns.sum())\n",
   "  g([v], b: [0], p: [], q: [1], same: fn(ys) -> ys, k: fn(ys) -> ys.len)"),
 # A HOF inside another HOF's callback argument.
 "nested_map_in_fold": (f"g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let n = [[{D}]].fold(initial: 0, step: fn(acc, row) -> acc + row.map(each: k).sum())\n  @log.write(n)\n",
   "  g([v], b: [0], p: [], q: [1], k: fn(ys) -> ys.len)"),
 # filter whose kept list is then folded by a public callback.
 "filter_then_fold": (f"g[T](a: T, b: T, p: T, q: T, keep: fn(T) -> Bool, step: fn(Int, T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let n = [{D}].filter(keep: keep).fold(initial: 0, step: step)\n  @log.write(n)\n",
   "  g([v], b: [0], p: [], q: [1], keep: fn(ys) -> true, step: fn(acc, ys) -> acc + ys.len)"),
 # An effectful callback: a function value whose type uses Log, applied by map under the decision.
 "map_effectful_callback": (f"g[T](a: T, b: T, p: T, q: T, w: fn(T) -> Int uses @log: Log[write]) -> Unit\n  uses @log: Log[write]\n=\n  let ns = [{D}].map(each: w)\n  @log.write(0)\n",
   "  g([v], b: [0], p: [], q: [1], w: fn(ys) -> ys.len)"),
 # A failing callback, applied by map under the decision.
 "map_failing_callback": (f"pub kind Boom = Bang | Other\n\ng[T](a: T, b: T, p: T, q: T, z: fn(T) -> Int fails Boom) -> Unit\n  uses @log: Log[write]\n  fails Boom\n=\n  let ns = [{D}].map(each: z)\n    if error pass\n  @log.write(0)\n",
   "  g([v], b: [0], p: [], q: [1], z: fn(ys) -> ys.len)"),
 # A first_where whose found element goes to a function value.
 "first_where_then_k": (f"g[T](a: T, b: T, p: T, q: T, keep: fn(T) -> Bool, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let n = when [{D}].first_where(keep: keep)\n    is Some(x) -> k(x)\n    is None -> 0\n  @log.write(n)\n",
   "  g([v], b: [0], p: [], q: [1], keep: fn(ys) -> true, k: fn(ys) -> ys.len)"),
 # Control: no decision, every HOF, must stay accepted.
 "ok_no_decision": ("g[T](p: T, q: T, k: fn(T) -> Int, keep: fn(T) -> Bool, step: fn(Int, T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let a = [p, q].map(each: k).sum()\n  let b = [p, q].filter(keep: keep).len\n  let c = [p, q].fold(initial: 0, step: step)\n  let d = when [p, q].first_where(keep: keep)\n    is Some(x) -> 1\n    is None -> 0\n  @log.write(a + b + c + d)\n",
   "  g([v], q: [1], k: fn(ys) -> ys.len, keep: fn(ys) -> ys.len > 0, step: fn(acc, ys) -> acc + ys.len)"),
}
for name, (g, call) in cases.items():
    d = os.path.join("cases", name)
    os.makedirs(os.path.join(d, "src"), exist_ok=True)
    open(os.path.join(d, "una.toml"), "w").write(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "0.1"\n\n[run]\nentry = "sw"\ngrants = ["console", "env"]\n')
    open(os.path.join(d, "verification.lock"), "w").write(open(LOCK).read())
    open(os.path.join(d, "src", "sw.una"), "w").write(HEAD + "\n" + g + MAIN + call + "\n")
