"""Edition 0.2 matrix for the prelude-HOF class of #253 (the v0.3.0 question): each case is a
generic `g` whose function-value callback is applied, by the prelude or a loop, to values
decided by comparing values of T, with a runnable `main` whose Log handler prints. Writes
one package per case under ./cases/<name>/. A case leaks when `check` accepts it and `run`
prints different numbers for S="" and S="x"."""
import os, sys
LOCK = sys.argv[1]
HEAD = '''module sw

pub capability Log
  op write(n: Int) -> Unit

pub handler to_console() for Log
  uses @console: Console[print]
  op write(n) = @console.print("{n}")

pub kind Box[T]
  run: fn(T) -> Int
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
 "filter": (f"g[T](a: T, b: T, p: T, q: T, keep: fn(T) -> Bool) -> Unit\n  uses @log: Log[write]\n=\n  let kept = [{D}].filter(keep: keep)\n  @log.write(kept.len)\n",
            "  g([v], b: [0], p: [], q: [1], keep: fn(xs) -> xs.len > 0)"),
 "first_where": (f"g[T](a: T, b: T, p: T, q: T, pred: fn(T) -> Bool) -> Unit\n  uses @log: Log[write]\n=\n  let found = [{D}].first_where(keep: pred)\n  let n = when found\n    is Some(x) -> 1\n    is None -> 0\n  @log.write(n)\n",
            "  g([v], b: [0], p: [], q: [1], pred: fn(xs) -> xs.len > 0)"),
 "fold_named": (f"g[T](a: T, b: T, p: T, q: T, step: fn(Int, T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let n = [{D}].fold(initial: 0, step: step)\n  @log.write(n)\n",
            "  g([v], b: [0], p: [], q: [1], step: fn(acc, xs) -> acc + xs.len)"),
 "fold_lambda_k": (f"g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let n = [{D}].fold(initial: 0, step: fn(acc, x) -> acc + k(x))\n  @log.write(n)\n",
            "  g([v], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)"),
 "map_wrapped_lambda": (f"g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let ns = [{D}].map(each: fn(x) -> k(x))\n  @log.write(ns.sum())\n",
            "  g([v], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)"),
 "map_let_bound": (f"g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let f = k\n  let ns = [{D}].map(each: f)\n  @log.write(ns.sum())\n",
            "  g([v], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)"),
 "map_record_callback": (f"g[T](a: T, b: T, p: T, q: T, box: Box[T]) -> Unit\n  uses @log: Log[write]\n=\n  let ns = [{D}].map(each: box.run)\n  @log.write(ns.sum())\n",
            "  g([v], b: [0], p: [], q: [1], box: Box(run: fn(xs) -> xs.len))"),
 "map_list_callback": (f"g[T](a: T, b: T, p: T, q: T, ks: List[fn(T) -> Int]) -> Unit\n  uses @log: Log[write]\n=\n  let n = when ks.first()\n    is Some(f) -> [{D}].map(each: f).sum()\n    is None -> 0\n  @log.write(n)\n",
            "  g([v], b: [0], p: [], q: [1], ks: [fn(xs) -> xs.len])"),
 "entries_fold": (f"g[T](a: T, b: T, p: T, q: T, step: fn(Int, Entry[Int, T]) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let m = empty_map().with(key: 0, value: {D})\n  let n = m.entries().fold(initial: 0, step: step)\n  @log.write(n)\n",
            "  g([v], b: [0], p: [], q: [1], step: fn(acc, e) -> acc + e.value.len)"),
 "values_map": (f"g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let m = empty_map().with(key: 0, value: {D})\n  let ns = m.values().map(each: k)\n  @log.write(ns.sum())\n",
            "  g([v], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)"),
 "text_fold": ("g[T](a: T, b: T, step: fn(Int, Text) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let e = \"\"\n  let x = \"x\"\n  let n = [if a == b then e else x].fold(initial: 0, step: step)\n  @log.write(n)\n",
            "  g([v], b: [0], step: fn(acc, t) -> acc + t.len)"),
 "for_loop_k": (f"g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let edit n = 0\n  for x in [{D}]\n    n = n + k(x)\n  @log.write(n)\n",
            "  g([v], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)"),
 "together_for_k": (f"g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit\n  uses @log: Log[write]\n=\n  let ns = together for x in [{D}]\n    k(x)\n  @log.write(ns.sum())\n",
            "  g([v], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)"),
}
for name, (g, call) in cases.items():
    d = os.path.join("cases", name)
    os.makedirs(os.path.join(d, "src"), exist_ok=True)
    open(os.path.join(d, "una.toml"), "w").write(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "0.2"\n\n[run]\nentry = "sw"\ngrants = ["console", "env"]\n')
    open(os.path.join(d, "verification.lock"), "w").write(open(LOCK).read())
    open(os.path.join(d, "src", "sw.una"), "w").write(HEAD + "\n" + g + MAIN + call + "\n")
