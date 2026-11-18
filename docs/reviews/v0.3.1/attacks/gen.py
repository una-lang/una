# Generates the attack cases of the #349 review: one package per case, edition 0.1 unless noted.
import os, sys
root = os.path.dirname(os.path.abspath(__file__)) + "/cases"
LOCK = '''encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
'''
HEAD = '''module sw

pub capability Log
  op write(n: Int) -> Unit

'''
LEAK = '''
pub leak(s: secret Int) -> Unit
  uses @log: Log[write]
=
  g([s], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)
'''
SIG = '''g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit
  uses @log: Log[write]
=
'''
cases = {}
def case(name, expect, body, edition="0.1", whole=None):
    cases[name] = (expect, edition, whole if whole else HEAD + body)

# 1. User-defined generic HOFs.
case("user_map_prelude_inside", "U4003", '''my_map[U](xs: List[U], f: fn(U) -> Int) -> List[Int] = xs.map(each: f)

''' + SIG + '''  let ns = my_map([if a == b then p else q], f: k)
  @log.write(ns.sum())
''' + LEAK)
case("user_map_recursive", "U4003", '''my_map[U](xs: List[U], f: fn(U) -> Int) -> List[Int] = xs.fold(initial: [], step: fn(acc, x) -> acc.append(f(x)))

''' + SIG + '''  let ns = my_map([if a == b then p else q], f: k)
  @log.write(ns.sum())
''' + LEAK)
case("user_apply_direct", "U4003", '''apply[U](x: U, f: fn(U) -> Int) -> Int = f(x)

''' + SIG + '''  @log.write(apply(if a == b then p else q, f: k))
''' + LEAK)
case("user_nested_hof", "U4003", '''inner[U](xs: List[U], f: fn(U) -> Int) -> Int = xs.map(each: f).sum()

outer[V](xs: List[V], f: fn(V) -> Int) -> Int = inner(xs, f: f)

''' + SIG + '''  @log.write(outer([if a == b then p else q], f: k))
''' + LEAK)
case("user_hof_non_generic_int", "U4003", '''total(xs: List[Int], f: fn(Int) -> Int) -> Int = xs.map(each: f).sum()

g[T](a: T, b: T, h: fn(Int) -> Int) -> Unit
  uses @log: Log[write]
=
  @log.write(total([if a == b then 1 else 0], f: h))

pub leak(s: secret Int) -> Unit
  uses @log: Log[write]
=
  g(s, b: 0, h: fn(n) -> n)
''')
case("user_hof_shape_decided", "U4003", '''count_all[U](xs: List[U], f: fn(U) -> Int) -> Int = xs.map(each: f).sum()

''' + SIG + '''  @log.write(count_all(if a == b then [p] else [p, q], f: k))
''' + LEAK)
case("user_hof_returns_callback", "U4003", '''pick[U](x: U, f: fn(U) -> Int) -> fn(U) -> Int = f

''' + SIG + '''  let d = if a == b then p else q
  let h = pick(d, f: k)
  @log.write([d].map(each: h).sum())
''' + LEAK)
case("ok_user_hof_no_decision", "ok", '''my_map[U](xs: List[U], f: fn(U) -> Int) -> List[Int] = xs.map(each: f)

g(a: Int, k: fn(Int) -> Int) -> Unit
  uses @log: Log[write]
=
  @log.write(my_map([a, 2], f: k).sum())

pub run() -> Unit
  uses @log: Log[write]
=
  g(1, k: fn(n) -> n)
''')

# 3. Partial decisions.
case("partial_fold_initial_decided_named", "U4003", SIG + '''  let n = [p].fold(initial: if a == b then 1 else 0, step: fn(acc, x) -> acc + k(x))
  @log.write(n)
''' + LEAK)
case("partial_fold_step_named_two_params", "U4003", '''g[T](a: T, b: T, p: T, q: T, k: fn(Int, T) -> Int) -> Unit
  uses @log: Log[write]
=
  @log.write([p, q].fold(initial: if a == b then 1 else 0, step: k))

pub leak(s: secret Int) -> Unit
  uses @log: Log[write]
=
  g([s], b: [0], p: [], q: [1], k: fn(n, xs) -> n)
''')
case("partial_fold_decided_elements_lambda_takes_acc_only", "U4003", '''g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int) -> Unit
  uses @log: Log[write]
=
  let d = if a == b then 1 else 0
  @log.write([if a == b then p else q].fold(initial: d, step: fn(acc, x) -> k(x)))
''' + LEAK)
case("partial_named_args_step_first", "U4003", '''g[T](a: T, b: T, p: T, q: T, h: fn(Int) -> Int) -> Unit
  uses @log: Log[write]
=
  @log.write([p].fold(step: fn(acc, x) -> h(acc), initial: if a == b then 1 else 0))
''' + LEAK.replace("k: fn(xs) -> xs.len", "h: fn(n) -> n"))
case("partial_let_lambda_before_use", "U4003", SIG + '''  let f = fn(x: T) -> k(x)
  @log.write([if a == b then p else q].map(each: f).sum())
''' + LEAK)
case("partial_two_type_params", "U4003", '''g[T, V](a: T, b: T, c: V, e: V, p: T, k: fn(T) -> Int) -> Unit
  uses @log: Log[write]
=
  let n = if c == e then 1 else 0
  let pairs = [if a == b then p else p]
  @log.write(pairs.fold(initial: n, step: fn(acc, x) -> k(x)))

pub leak(s: secret Int) -> Unit
  uses @log: Log[write]
=
  g([1], b: [0], c: s, e: 0, p: [], k: fn(xs) -> xs.len)
''')
case("partial_filter_lambda_ignores", "U4003", SIG + '''  let kept = [p, q].filter(keep: fn(x) -> a == b)
  @log.write(kept.map(each: k).sum())
''' + LEAK)
case("partial_one_of_two_lists", "U4003", SIG + '''  let xs = [p]
  let ys = [if a == b then p else q]
  @log.write(xs.map(each: k).sum() + ys.map(each: k).sum())
''' + LEAK)
case("partial_decided_function_value", "U4003", '''g[T](a: T, b: T, p: T, k1: fn(T) -> Int, k2: fn(T) -> Int) -> Unit
  uses @log: Log[write]
=
  @log.write([p].map(each: if a == b then k1 else k2).sum())

pub leak(s: secret Int) -> Unit
  uses @log: Log[write]
=
  g(s, b: 0, p: 0, k1: fn(n) -> 1, k2: fn(n) -> 0)
''')
case("partial_option_list", "U4003", SIG + '''  let o = if a == b then Some(p) else None
  let n = when o
    is Some(x) -> [x].map(each: k).sum()
    is None -> 0
  @log.write(n)
''' + LEAK)
case("partial_list_of_lists", "U4003", SIG + '''  let xss = [if a == b then [p] else [p, q]]
  @log.write(xss.map(each: fn(xs) -> xs.map(each: k).sum()).sum())
''' + LEAK)
case("partial_contains_callback", "U4003", SIG + '''  let found = [p, q].first_where(keep: fn(x) -> x == a)
  let n = when found
    is Some(x) -> k(x)
    is None -> 0
  @log.write(n)
''' + LEAK)
case("ok_lambda_ignores_decided", "ok", SIG + '''  @log.write([if a == b then p else q].map(each: fn(x) -> 1).sum())
''' + LEAK)
case("ok_named_without_decision", "ok", SIG + '''  @log.write([p, q].map(each: k).sum())
''' + LEAK)

# 4. Effects and failures.
EFF_SIG = """g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Unit uses @log: Log[write]) -> Unit
  uses @log: Log[write]
=
"""
EFF_LEAK = """
pub leak(s: secret Int) -> Unit
  uses @log: Log[write]
=
  g([s], b: [0], p: [], q: [1], k: fn(xs) -> @log.write(xs.len))
"""
case("effect_direct_named_callback", "U4002", EFF_SIG + """  k(if a == b then p else q)
""" + EFF_LEAK)
case("effect_user_hof_named_callback", "U4002", '''apply_eff[U](x: U, f: fn(U) -> Unit uses @log: Log[write]) -> Unit
  uses @log: Log[write]
=
  f(x)

''' + EFF_SIG + """  apply_eff(if a == b then p else q, f: k)
""" + EFF_LEAK)
case("effect_user_hof_nested", "U4002", '''apply_eff[U](x: U, f: fn(U) -> Unit uses @log: Log[write]) -> Unit
  uses @log: Log[write]
=
  f(x)

twice[V](x: V, f: fn(V) -> Unit uses @log: Log[write]) -> Unit
  uses @log: Log[write]
=
  apply_eff(x, f: f)

''' + EFF_SIG + """  twice(if a == b then p else q, f: k)
""" + EFF_LEAK)
case("effect_prelude_map_named_callback", "U3003", EFF_SIG + """  let units = [if a == b then p else q].map(each: k)
  @log.write(units.len)
""" + EFF_LEAK)
case("failing_prelude_map_callback", "U3009", '''kind Bad = | Oops

g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int fails Bad) -> Unit
  uses @log: Log[write]
=
  @log.write([if a == b then p else q].map(each: k).sum())

pub leak(s: secret Int) -> Unit
  uses @log: Log[write]
=
  g([s], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)
''')
case("failing_user_hof_callback", "U4002", '''kind Bad = | Oops

apply_f[U](x: U, f: fn(U) -> Int fails Bad) -> Int
  fails Bad
=
  f(x)

g[T](a: T, b: T, p: T, q: T, k: fn(T) -> Int fails Bad) -> Unit
  uses @log: Log[write]
=
  let n = attempt apply_f(if a == b then p else q, f: k)
  @log.write(1)

pub leak(s: secret Int) -> Unit
  uses @log: Log[write]
=
  g([s], b: [0], p: [], q: [1], k: fn(xs) -> xs.len)
''')

# 2. sort: edition 0.2 only.
case("sort_in_0_1", "unknown", SIG + '''  @log.write([1, 2].sort().sum())
''' + LEAK)
case("sort_0_2_decided", "U4001 or U4003", SIG + '''  @log.write([if a == b then 1 else 0, 2].sort().sum())
''' + LEAK, edition="0.2")

for name, (expect, edition, src) in cases.items():
    d = f"{root}/{name}"
    os.makedirs(d + "/src", exist_ok=True)
    open(d + "/una.toml", "w").write(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "{edition}"\n')
    open(d + "/verification.lock", "w").write(LOCK)
    open(d + "/src/sw.una", "w").write(f"-- expect: {expect}\n" + src)
print(len(cases))
